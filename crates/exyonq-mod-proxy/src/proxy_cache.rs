//! Plan 12 — bounded proxy response materialization for microcache (KD3.5 module ownership).

use crate::headers::{parse_response_content_length, ContentLengthParse};
use crate::hyper_forward::{
    bad_gateway, forward_get_streaming, response_is_event_stream, ProxyHyperMetrics,
};
use crate::UpstreamTarget;
use bytes::Bytes;
use exyonq_module_api::{assess_cacheability, CacheRejection};
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame};
use hyper::header::HeaderValue;
use hyper::{Response, StatusCode};
use std::pin::Pin;
use std::task::{Context, Poll};

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

/// Internal detail for bounded materialization outcomes (not exported as metrics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaterializationDetail {
    OversizedKnownLength,
    OversizedWhileStreaming,
    SseOrStreaming,
    BodyReadError,
    ContentLengthInvalid,
    ContentLengthMismatch,
}

/// Result of bounded proxy body materialization for cache coordination.
pub enum ProxyCacheLoad {
    Materialized {
        body: Bytes,
        status: StatusCode,
        headers: Vec<(String, String)>,
        cacheable: bool,
        rejection: Option<CacheRejection>,
    },
    Passthrough {
        response: Response<BoxBody>,
        rejection: CacheRejection,
    },
}

/// Streaming GET upstream fetch + bounded materialization for kernel cache insert path.
pub async fn load_get_for_cache(
    upstream: &UpstreamTarget,
    path_and_query: &str,
    x_forwarded_for: Option<&HeaderValue>,
    max_object_bytes: usize,
    request_headers: &[(String, String)],
    metrics: &ProxyHyperMetrics,
) -> ProxyCacheLoad {
    let response = forward_get_streaming(upstream, path_and_query, x_forwarded_for, metrics).await;
    prepare_proxy_cache_load(response, max_object_bytes, request_headers).await
}

/// Materialize a proxy response for optional cache insert (never truncates the client body).
pub async fn prepare_proxy_cache_load(
    response: Response<BoxBody>,
    max_object_bytes: usize,
    request_headers: &[(String, String)],
) -> ProxyCacheLoad {
    let (parts, body) = response.into_parts();
    if response_is_event_stream(&parts) {
        return passthrough(
            parts,
            body.boxed(),
            CacheRejection::ResponseStreaming,
            MaterializationDetail::SseOrStreaming,
        );
    }

    let headers = header_pairs(&parts);

    let declared_len = match parse_response_content_length(&parts.headers) {
        ContentLengthParse::Invalid => {
            return passthrough(
                parts,
                body.boxed(),
                CacheRejection::BodyNotMaterialized,
                MaterializationDetail::ContentLengthInvalid,
            );
        }
        ContentLengthParse::Valid(len) if len > max_object_bytes => {
            return passthrough(
                parts,
                body.boxed(),
                CacheRejection::BodyTooLarge,
                MaterializationDetail::OversizedKnownLength,
            );
        }
        ContentLengthParse::Valid(len) => Some(len),
        ContentLengthParse::Absent => None,
    };

    match read_bounded_body(body, max_object_bytes).await {
        BoundedReadOutcome::Complete(body) => {
            if let Some(cl) = declared_len {
                if body.len() != cl {
                    let _detail = MaterializationDetail::ContentLengthMismatch;
                    return materialized(
                        parts.status,
                        &headers,
                        body,
                        false,
                        Some(CacheRejection::BodyNotMaterialized),
                    );
                }
            }
            let assessment = assess_cacheability(
                "GET",
                request_headers,
                parts.status.as_u16(),
                &headers,
                body.len(),
                max_object_bytes,
            );
            materialized(
                parts.status,
                &headers,
                body,
                assessment.is_ok(),
                assessment.err(),
            )
        }
        BoundedReadOutcome::Oversized {
            prefix,
            first_chunk,
            remainder,
        } => {
            let _detail = MaterializationDetail::OversizedWhileStreaming;
            let chained = chain_prefix_and_body(prefix, first_chunk, remainder);
            passthrough(
                parts,
                chained,
                CacheRejection::BodyTooLarge,
                MaterializationDetail::OversizedWhileStreaming,
            )
        }
        BoundedReadOutcome::ReadError => {
            let _detail = MaterializationDetail::BodyReadError;
            ProxyCacheLoad::Passthrough {
                response: bad_gateway(),
                rejection: CacheRejection::BodyNotMaterialized,
            }
        }
    }
}

enum BoundedReadOutcome {
    Complete(Bytes),
    Oversized {
        prefix: Bytes,
        first_chunk: Bytes,
        remainder: BoxBody,
    },
    ReadError,
}

async fn read_bounded_body(body: BoxBody, max_object_bytes: usize) -> BoundedReadOutcome {
    let cap = max_object_bytes.saturating_add(1);
    let mut buf = Vec::with_capacity(cap.min(4096));
    let mut body = body;

    loop {
        match body.frame().await {
            None => return BoundedReadOutcome::Complete(Bytes::from(buf)),
            Some(Err(_)) => return BoundedReadOutcome::ReadError,
            Some(Ok(frame)) => {
                let Ok(chunk) = frame.into_data() else {
                    continue;
                };
                if chunk.is_empty() {
                    continue;
                }
                let room = cap.saturating_sub(buf.len());
                if chunk.len() <= room {
                    buf.extend_from_slice(&chunk);
                    if buf.len() >= cap {
                        match body.frame().await {
                            None => return BoundedReadOutcome::Complete(Bytes::from(buf)),
                            Some(Err(_)) => return BoundedReadOutcome::ReadError,
                            Some(Ok(next)) => {
                                let Ok(tail) = next.into_data() else {
                                    return BoundedReadOutcome::Oversized {
                                        prefix: Bytes::from(buf),
                                        first_chunk: Bytes::new(),
                                        remainder: body.boxed(),
                                    };
                                };
                                return BoundedReadOutcome::Oversized {
                                    prefix: Bytes::from(buf),
                                    first_chunk: tail,
                                    remainder: body.boxed(),
                                };
                            }
                        }
                    }
                    continue;
                }
                buf.extend_from_slice(&chunk[..room]);
                let first_chunk = chunk.slice(room..);
                return BoundedReadOutcome::Oversized {
                    prefix: Bytes::from(buf),
                    first_chunk,
                    remainder: body.boxed(),
                };
            }
        }
    }
}

fn chain_prefix_and_body(prefix: Bytes, first_chunk: Bytes, remainder: BoxBody) -> BoxBody {
    PrefixThenBody {
        prefix: Some(prefix),
        first_chunk: Some(first_chunk),
        remainder: Some(remainder),
    }
    .boxed()
}

struct PrefixThenBody {
    prefix: Option<Bytes>,
    first_chunk: Option<Bytes>,
    remainder: Option<BoxBody>,
}

impl Body for PrefixThenBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if let Some(prefix) = this.prefix.take() {
            if !prefix.is_empty() {
                return Poll::Ready(Some(Ok(Frame::data(prefix))));
            }
        }
        if let Some(first) = this.first_chunk.take() {
            if !first.is_empty() {
                return Poll::Ready(Some(Ok(Frame::data(first))));
            }
        }
        let Some(body) = this.remainder.as_mut() else {
            return Poll::Ready(None);
        };
        match Pin::new(body).poll_frame(cx) {
            Poll::Ready(None) => {
                this.remainder = None;
                Poll::Ready(None)
            }
            Poll::Ready(Some(result)) => Poll::Ready(Some(result)),
            Poll::Pending => Poll::Pending,
        }
    }
}

fn header_pairs(parts: &http::response::Parts) -> Vec<(String, String)> {
    parts
        .headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect()
}

fn materialized(
    status: StatusCode,
    headers: &[(String, String)],
    body: Bytes,
    cacheable: bool,
    rejection: Option<CacheRejection>,
) -> ProxyCacheLoad {
    ProxyCacheLoad::Materialized {
        body,
        status,
        headers: headers.to_vec(),
        cacheable,
        rejection,
    }
}

fn passthrough(
    parts: http::response::Parts,
    body: BoxBody,
    rejection: CacheRejection,
    _detail: MaterializationDetail,
) -> ProxyCacheLoad {
    ProxyCacheLoad::Passthrough {
        response: Response::from_parts(parts, body),
        rejection,
    }
}

pub fn build_materialized_response(
    status: StatusCode,
    headers: &[(String, String)],
    body: Bytes,
) -> Response<BoxBody> {
    let mut builder = Response::builder().status(status);
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("transfer-encoding") {
            continue;
        }
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder
        .header("content-length", body.len())
        .body(Full::from(body).map_err(|never| match never {}).boxed())
        .expect("valid materialized proxy response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::{BodyExt, Full};
    use hyper::header::{CONTENT_LENGTH, CONTENT_TYPE};

    fn full_response(
        status: StatusCode,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Response<BoxBody> {
        let mut builder = Response::builder().status(status);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder
            .body(
                Full::from(Bytes::copy_from_slice(body))
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .expect("response")
    }

    #[tokio::test]
    async fn sse_passthrough_not_materialized_or_cached() {
        let response = full_response(
            StatusCode::OK,
            &[(CONTENT_TYPE.as_str(), "text/event-stream")],
            b"data: hello\n\n",
        );
        let load = prepare_proxy_cache_load(response, 1024, &[]).await;
        match load {
            ProxyCacheLoad::Passthrough { rejection, .. } => {
                assert_eq!(rejection, CacheRejection::ResponseStreaming);
            }
            ProxyCacheLoad::Materialized { .. } => panic!("SSE must not materialize"),
        }
    }

    #[tokio::test]
    async fn small_body_materializes_and_assesses_cacheability() {
        let response = full_response(StatusCode::OK, &[], b"ok");
        let load = prepare_proxy_cache_load(response, 1024, &[]).await;
        match load {
            ProxyCacheLoad::Materialized {
                body,
                cacheable,
                rejection,
                ..
            } => {
                assert_eq!(&body[..], b"ok");
                assert!(cacheable);
                assert!(rejection.is_none());
            }
            ProxyCacheLoad::Passthrough { .. } => panic!("expected materialized"),
        }
    }

    #[tokio::test]
    async fn oversized_known_length_passthrough() {
        let response = full_response(StatusCode::OK, &[(CONTENT_LENGTH.as_str(), "2048")], b"x");
        let load = prepare_proxy_cache_load(response, 64, &[]).await;
        match load {
            ProxyCacheLoad::Passthrough { rejection, .. } => {
                assert_eq!(rejection, CacheRejection::BodyTooLarge);
            }
            ProxyCacheLoad::Materialized { .. } => panic!("expected passthrough"),
        }
    }

    #[tokio::test]
    async fn auth_request_bypasses_cache_on_materialized_path() {
        let response = full_response(StatusCode::OK, &[], b"secret");
        let headers = vec![("authorization".into(), "Bearer x".into())];
        let load = prepare_proxy_cache_load(response, 1024, &headers).await;
        match load {
            ProxyCacheLoad::Materialized {
                cacheable,
                rejection,
                ..
            } => {
                assert!(!cacheable);
                assert_eq!(rejection, Some(CacheRejection::RequestAuthorization));
            }
            ProxyCacheLoad::Passthrough { .. } => panic!("expected materialized bypass"),
        }
    }
}
