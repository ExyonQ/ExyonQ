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
//! Minimal single-chunk body for preloaded static assets.

use bytes::Bytes;
use hyper::body::{Body, Frame, SizeHint};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

pub type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

/// One-shot body backed by shared bytes (refcount clone on poll, no buffer copy).
pub struct StaticBody {
    data: Option<Arc<Bytes>>,
}

impl StaticBody {
    pub fn from_arc(data: Arc<Bytes>) -> Self {
        Self { data: Some(data) }
    }
}

impl Body for StaticBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.data.take() {
            None => Poll::Ready(None),
            Some(arc) => Poll::Ready(Some(Ok(Frame::data((*arc).clone())))),
        }
    }

    fn is_end_stream(&self) -> bool {
        self.data.is_none()
    }

    fn size_hint(&self) -> SizeHint {
        let mut hint = SizeHint::new();
        if let Some(ref data) = self.data {
            hint.set_exact(data.len() as u64);
        }
        hint
    }
}
