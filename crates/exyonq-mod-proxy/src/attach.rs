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
//! Pending streaming/WebSocket hyper responses keyed by opaque handle (KD3.2 attach seam).

use exyonq_module_api::proxy_dispatch::{ProxyStreamHandle, ProxyWebSocketHandle};
use http_body_util::combinators::BoxBody;
use hyper::Response;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

type LiveResponse = Response<BoxBody<bytes::Bytes, hyper::Error>>;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static PENDING_STREAMS: Mutex<Option<HashMap<u64, LiveResponse>>> = Mutex::new(None);
static PENDING_WS: Mutex<Option<HashMap<u64, LiveResponse>>> = Mutex::new(None);

fn stream_map() -> std::sync::MutexGuard<'static, Option<HashMap<u64, LiveResponse>>> {
    let mut guard = PENDING_STREAMS.lock().expect("stream registry poisoned");
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    guard
}

fn ws_map() -> std::sync::MutexGuard<'static, Option<HashMap<u64, LiveResponse>>> {
    let mut guard = PENDING_WS.lock().expect("ws registry poisoned");
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    guard
}

pub fn store_streaming_response(response: LiveResponse) -> ProxyStreamHandle {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    stream_map()
        .as_mut()
        .expect("stream map")
        .insert(id, response);
    ProxyStreamHandle(id)
}

pub fn take_streaming_response(handle: ProxyStreamHandle) -> Option<LiveResponse> {
    stream_map().as_mut().expect("stream map").remove(&handle.0)
}

pub fn store_websocket_response(response: LiveResponse) -> ProxyWebSocketHandle {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    ws_map().as_mut().expect("ws map").insert(id, response);
    ProxyWebSocketHandle(id)
}

pub fn take_websocket_response(handle: ProxyWebSocketHandle) -> Option<LiveResponse> {
    ws_map().as_mut().expect("ws map").remove(&handle.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::{BodyExt, Full};

    fn empty_200() -> LiveResponse {
        Response::builder()
            .status(200)
            .body(
                Full::from(bytes::Bytes::new())
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .expect("response")
    }

    #[test]
    fn streaming_handle_roundtrip() {
        let handle = store_streaming_response(empty_200());
        assert!(take_streaming_response(handle).is_some());
        assert!(take_streaming_response(handle).is_none());
    }
}
