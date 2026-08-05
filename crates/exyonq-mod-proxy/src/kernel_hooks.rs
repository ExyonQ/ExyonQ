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
//! Install module-api proxy wire hooks (KD3.4 composition root).

use crate::runtime::ProxyRuntime;
use crate::wire_conn;
use exyonq_module_api::proxy_wire::{self, ProxyWireAsyncHooks, ProxyWireEligibilityHooks};
use std::sync::Arc;

/// Register stable kernel hooks after `ProxyRuntime` is constructed.
pub fn install_kernel_hooks(runtime: Arc<ProxyRuntime>) {
    wire_conn::pin_runtime(Arc::clone(&runtime));

    let _ = proxy_wire::install_wire_eligibility_hooks(ProxyWireEligibilityHooks {
        might_use_proxy_wire: wire_conn::might_use_proxy_wire,
    });

    let _ = proxy_wire::install_wire_async_hooks(ProxyWireAsyncHooks {
        serve_proxy_wire: serve_proxy_wire_hook,
    });
}

fn serve_proxy_wire_hook(
    cluster_id: u32,
    generation: u64,
    x_forwarded_for: String,
    stream: proxy_wire::BoxedWireStream,
    head: bytes::Bytes,
    rest: bytes::Bytes,
) -> proxy_wire::ProxyWireServeFuture {
    let xff = hyper::header::HeaderValue::from_str(&x_forwarded_for)
        .unwrap_or_else(|_| hyper::header::HeaderValue::from_static("0.0.0.0"));
    Box::pin(async move {
        wire_conn::serve(
            cluster_id,
            generation,
            crate::wire_stream::into_async_io(stream),
            head,
            rest,
            xff,
        )
        .await
    })
}
