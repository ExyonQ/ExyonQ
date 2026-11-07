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

/// Bytes re-presented from the tail of the already-inspected region so a
/// signature straddling the seam between two inspections is still offered
/// whole. Anchored ops (`^`, `StartsWith`, `EndsWith`) evaluate on the
/// fragment — the same chunk-boundary contract the request path ships
/// (`inspect_body_chunk` feeds raw chunks, no prefix re-scan).
const RESPONSE_BODY_SCAN_OVERLAP: usize = 4096;

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
    /// AUDIT-D4: watermark — bytes already submitted to the engine. Each
    /// inspection covers only `[inspected - OVERLAP, buffer.len())`: the old
    /// code re-inspected the full accumulated prefix per chunk forever
    /// (quadratic; ~6 GiB scanned on a 100 MiB response at the 1 MiB cap).
    inspected: usize,
    limit_reached: bool,
    done: bool,
    error: Option<BoxError>,
}

impl WafBody {
    fn inspect(&mut self, final_chunk: bool) -> Result<(), BoxError> {
        let _ = final_chunk;
        if self.limit_reached {
            return Ok(());
        }
        let inspect_len = if self.max_bytes == 0 {
            self.buffer.len()
        } else {
            self.buffer.len().min(self.max_bytes)
        };
        if self.inspected >= inspect_len {
            return Ok(());
        }
        let start = self.inspected.saturating_sub(RESPONSE_BODY_SCAN_OVERLAP);
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
            body: Some(&self.buffer[start..inspect_len]),
        };
        let started = std::time::Instant::now();
        let decision = self
            .engine
            .inspect_response(WafPhase::ResponseBodyChunk, &req, &resp);
        crate::waf_metrics::note_inspection(WafPhase::ResponseBodyChunk, started.elapsed());
        crate::waf_metrics::note_decision(&decision);
        self.inspected = inspect_len;
        if decision.action == exyonq_waf_api::WafAction::Block {
            return Err(Box::new(WafBodyError("waf response body blocked")));
        }
        // AUDIT-D4: `> max_bytes` was unreachable (the poll loop caps the
        // buffer at max_bytes), so `limit_reached` was dead code and the
        // rescan never stopped. `>=` fires exactly when the configured
        // window is full — every inspectable byte has been submitted.
        if self.max_bytes > 0 && self.buffer.len() >= self.max_bytes {
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
        inspected: 0,
        limit_reached: false,
        done: false,
        error: None,
    }
    .boxed();
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_waf_api::{BodyInspectionState, WafDecision};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// One ready data frame per poll — no async starvation involved.
    struct ChunkedBody {
        chunks: VecDeque<Bytes>,
    }

    impl Body for ChunkedBody {
        type Data = Bytes;
        type Error = BoxError;

        fn poll_frame(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            match self.get_mut().chunks.pop_front() {
                Some(chunk) => Poll::Ready(Some(Ok(Frame::data(chunk)))),
                None => Poll::Ready(None),
            }
        }
    }

    /// Records the size of every body slice submitted to the engine.
    struct CountingEngine {
        slices: Mutex<Vec<usize>>,
    }

    impl WafEngine for CountingEngine {
        fn inspect(&self, _phase: WafPhase, _req: &WafRequest<'_>) -> WafDecision {
            WafDecision::allow()
        }

        fn inspect_body_chunk(
            &self,
            _req: &WafRequest<'_>,
            _chunk: &[u8],
            _end_of_stream: bool,
            _state: &mut BodyInspectionState,
        ) -> WafDecision {
            WafDecision::allow()
        }

        fn inspects_response_body(&self) -> bool {
            true
        }

        fn inspect_response(
            &self,
            _phase: WafPhase,
            _req: &WafRequest<'_>,
            resp: &WafResponse<'_>,
        ) -> WafDecision {
            self.slices
                .lock()
                .expect("slices")
                .push(resp.body.map(|body| body.len()).unwrap_or(0));
            WafDecision::allow()
        }
    }

    fn drain(body: FinalBoxBody) -> usize {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("rt");
        rt.block_on(async move {
            let mut streamed = 0usize;
            let mut body = std::pin::pin!(body);
            while let Some(frame) = std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
                let frame = frame.expect("stream");
                if let Ok(data) = frame.into_data() {
                    streamed += data.len();
                }
            }
            streamed
        })
    }

    /// AUDIT-D4: streaming inspection must be incremental — new bytes plus a
    /// bounded overlap — and must stop entirely once the configured window
    /// saturates. The old code re-submitted the whole accumulated prefix per
    /// chunk forever (quadratic) and `limit_reached` was unreachable.
    #[test]
    fn scans_incrementally_and_stops_at_limit() {
        let engine = Arc::new(CountingEngine {
            slices: Mutex::new(Vec::new()),
        });
        let chunk = Bytes::from(vec![b'x'; 8 * 1024]);
        let chunks: VecDeque<Bytes> = (0..4).map(|_| chunk.clone()).collect();
        let mut response = hyper::Response::builder()
            .status(200)
            .body(ChunkedBody { chunks }.boxed())
            .expect("response");
        let max_bytes = 16 * 1024;
        maybe_wrap_response(
            engine.clone(),
            "GET",
            Some("example.test"),
            "/x",
            None,
            &[],
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            None,
            max_bytes,
            &mut response,
        );

        let streamed = drain(std::mem::replace(response.body_mut(), empty_body()));
        assert_eq!(streamed, 4 * 8 * 1024, "every byte still streams through");

        let slices = engine.slices.lock().expect("slices");
        // Old behaviour: 4 calls re-submitting the full prefix (8K,16K,16K,16K
        // = 56K scanned). Incremental: 2 calls — first chunk whole (8K), then
        // new bytes + 4K overlap as the window fills to the 16K cap (12K);
        // further chunks submit nothing once `limit_reached`.
        assert_eq!(slices.len(), 2, "engine calls: {slices:?}");
        assert!(
            slices.iter().all(|&len| len <= max_bytes),
            "no slice exceeds the inspection window: {slices:?}"
        );
        assert!(
            slices.iter().sum::<usize>() <= 3 * max_bytes / 2,
            "total scanned bytes bounded: {slices:?}"
        );
    }
}
