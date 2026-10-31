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
//! NG-18 / ADR-064 Phase C — streaming `sub_filter` response transform.
//!
//! Case-insensitive replacement across chunk boundaries, multiple rules,
//! `sub_filter_once`, MIME scoping and `Last-Modified` removal. Bounded output
//! per poll; never buffers the whole body.

use crate::gunzip::HttpBoxBody;
use bytes::{Buf, Bytes, BytesMut};
use exyonq_config_ir::{SubFilterConfig, SubFilterRouteConfig};
use http_body::{Body, Frame, SizeHint};
use http_body_util::BodyExt;
use std::pin::Pin;
use std::task::{Context, Poll};

const MAX_OUT_PER_POLL: usize = 32 * 1024;

/// Apply the route's `sub_filter` policy. The body must already be identity
/// (apply `gunzip` first); compressed passthrough is left untouched.
pub(crate) fn maybe_sub_filter(
    config: &SubFilterConfig,
    route_name: &str,
    response: &mut hyper::Response<HttpBoxBody>,
) {
    let Some(route) = config.routes.get(route_name) else {
        return;
    };
    if route.rules.is_empty() || response.status() != hyper::StatusCode::OK {
        return;
    }
    if response
        .headers()
        .get(hyper::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| !v.eq_ignore_ascii_case("identity"))
    {
        return;
    }
    let Some(content_type) = response
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
    else {
        return;
    };
    if !mime_allowed(content_type, &route.types) {
        return;
    }

    let headers = response.headers_mut();
    headers.remove(hyper::header::CONTENT_LENGTH);
    if !route.last_modified {
        headers.remove(hyper::header::LAST_MODIFIED);
    }

    let body = std::mem::replace(response.body_mut(), empty_body());
    *response.body_mut() = SubFilterBody::new(body, route).boxed();
}

fn empty_body() -> HttpBoxBody {
    http_body_util::Full::new(Bytes::new())
        .map_err(|never| match never {})
        .boxed()
}

fn mime_allowed(content_type: &str, types: &[String]) -> bool {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim();
    types.iter().any(|allowed| {
        let allowed = allowed.trim();
        allowed == "*" || allowed.eq_ignore_ascii_case(mime)
    })
}

struct Rule {
    pattern: Vec<u8>,
    replacement: Vec<u8>,
}

struct SubFilterBody {
    inner: HttpBoxBody,
    rules: Vec<Rule>,
    once: bool,
    buf: BytesMut,
    out: BytesMut,
    trailers: Option<hyper::HeaderMap>,
    done: bool,
    inner_done: bool,
    replaced_once: bool,
    error: Option<exyonq_module_api::BoxError>,
}

impl SubFilterBody {
    fn new(inner: HttpBoxBody, route: &SubFilterRouteConfig) -> Self {
        Self {
            inner,
            rules: route
                .rules
                .iter()
                .map(|r| Rule {
                    pattern: r.pattern.as_bytes().to_vec(),
                    replacement: r.replacement.as_bytes().to_vec(),
                })
                .collect(),
            once: route.once,
            buf: BytesMut::new(),
            out: BytesMut::new(),
            trailers: None,
            done: false,
            inner_done: false,
            replaced_once: false,
            error: None,
        }
    }

    fn feed(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
        self.process(false);
    }

    fn process(&mut self, final_chunk: bool) {
        let max_pattern = self
            .rules
            .iter()
            .map(|r| r.pattern.len())
            .max()
            .unwrap_or(1)
            .max(1);
        loop {
            if self.out.len() >= MAX_OUT_PER_POLL {
                return;
            }
            if self.buf.is_empty() {
                return;
            }
            if !final_chunk && self.buf.len() < max_pattern {
                return;
            }
            if self.once && self.replaced_once {
                let take = if final_chunk {
                    self.buf.len()
                } else {
                    self.buf.len().saturating_sub(max_pattern - 1)
                };
                if take == 0 {
                    return;
                }
                self.out.extend_from_slice(&self.buf[..take]);
                self.buf.advance(take);
                continue;
            }
            let found = find_match(&self.buf, &self.rules, final_chunk);
            match found {
                Some((idx, rule_idx, plen)) => {
                    self.out.extend_from_slice(&self.buf[..idx]);
                    self.buf.advance(idx);
                    self.out
                        .extend_from_slice(&self.rules[rule_idx].replacement);
                    self.buf.advance(plen);
                    if self.once {
                        self.replaced_once = true;
                    }
                }
                None => {
                    let take = if final_chunk {
                        self.buf.len()
                    } else {
                        self.buf.len().saturating_sub(max_pattern - 1)
                    };
                    if take == 0 {
                        return;
                    }
                    self.out.extend_from_slice(&self.buf[..take]);
                    self.buf.advance(take);
                }
            }
        }
    }
}

fn find_match(buf: &[u8], rules: &[Rule], final_chunk: bool) -> Option<(usize, usize, usize)> {
    let max_pattern = rules.iter().map(|r| r.pattern.len()).max().unwrap_or(1);
    if !final_chunk && buf.len() < max_pattern {
        return None;
    }
    for idx in 0..buf.len() {
        for (rule_idx, rule) in rules.iter().enumerate() {
            let end = idx + rule.pattern.len();
            if end > buf.len() {
                continue;
            }
            if buf[idx..end].eq_ignore_ascii_case(&rule.pattern) {
                return Some((idx, rule_idx, rule.pattern.len()));
            }
        }
    }
    None
}

impl Body for SubFilterBody {
    type Data = Bytes;
    type Error = exyonq_module_api::BoxError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        loop {
            if let Some(err) = this.error.take() {
                return Poll::Ready(Some(Err(err)));
            }
            if !this.out.is_empty() {
                return Poll::Ready(Some(Ok(Frame::data(this.out.split().freeze()))));
            }
            if this.done {
                return Poll::Ready(
                    this.trailers
                        .take()
                        .map(|trailers| Ok(Frame::trailers(trailers))),
                );
            }
            if this.inner_done {
                this.process(true);
                if this.out.is_empty() {
                    this.done = true;
                }
                continue;
            }
            if !this.buf.is_empty() {
                this.process(false);
                if !this.out.is_empty() {
                    continue;
                }
            }
            match Pin::new(&mut this.inner).poll_frame(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    this.inner_done = true;
                }
                Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
                    Ok(data) => this.feed(&data),
                    Err(frame) => {
                        if let Ok(trailers) = frame.into_trailers() {
                            this.trailers = Some(trailers);
                        }
                    }
                },
                Poll::Ready(Some(Err(err))) => return Poll::Ready(Some(Err(err))),
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.done && self.out.is_empty()
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_config_ir::SubFilterRule;
    use http_body_util::Full;

    fn boxed(data: Vec<u8>) -> HttpBoxBody {
        Full::new(Bytes::from(data))
            .map_err(|never| match never {})
            .boxed()
    }

    fn route(rules: &[(&str, &str)], once: bool) -> SubFilterRouteConfig {
        SubFilterRouteConfig {
            rules: rules
                .iter()
                .map(|(p, r)| SubFilterRule {
                    pattern: (*p).into(),
                    replacement: (*r).into(),
                })
                .collect(),
            once,
            types: vec!["text/html".into()],
            last_modified: false,
        }
    }

    fn collect(body: SubFilterBody) -> Vec<u8> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let mut out = Vec::new();
            let mut body = std::pin::pin!(body);
            while let Some(frame) = std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
                if let Ok(frame) = frame {
                    if let Ok(data) = frame.into_data() {
                        out.extend_from_slice(&data);
                    }
                }
            }
            out
        })
    }

    #[test]
    fn replaces_across_chunk_boundaries_case_insensitively() {
        let body = SubFilterBody::new(
            boxed(b"<html>HTTP://A HTTP://B</html>".to_vec()),
            &route(&[("http://", "https://")], false),
        );
        assert_eq!(collect(body), b"<html>https://A https://B</html>");
    }

    #[test]
    fn once_replaces_only_first() {
        let body = SubFilterBody::new(boxed(b"aXbXc".to_vec()), &route(&[("X", "Y")], true));
        assert_eq!(collect(body), b"aYbXc");
    }

    #[test]
    fn multiple_rules_earliest_wins() {
        let body = SubFilterBody::new(
            boxed(b"foo bar".to_vec()),
            &route(&[("bar", "BAR"), ("foo", "FOO")], false),
        );
        assert_eq!(collect(body), b"FOO BAR");
    }

    #[test]
    fn mime_matching() {
        assert!(mime_allowed(
            "text/html; charset=utf-8",
            &["text/html".into()]
        ));
        assert!(mime_allowed("application/json", &["*".into()]));
        assert!(!mime_allowed("text/css", &["text/html".into()]));
    }
}
