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
//! KD3 reverse-proxy module — metadata, Hyper runtime, dispatch service.

pub mod attach;
pub mod cache_metrics;
pub mod cache_serve;
pub mod errors;
pub mod forward;
pub mod headers;
pub mod hyper_client;
pub mod hyper_forward;
pub mod kernel_hooks;
pub mod proxy_cache;
pub mod runtime;
pub mod selector;
pub mod spike;
pub mod sse;
pub mod upstream;
pub mod upstream_target;
pub mod uri;
pub mod websocket;
pub mod wire_conn;
pub mod wire_io;
pub mod wire_stream;

#[cfg(feature = "p8-app-attribution")]
pub mod p8_app_attribution;

#[cfg(feature = "p8-transport-diag")]
pub(crate) mod streaming_wire_writer;

pub use cache_metrics::{
    cache_proxy_hits_total, cache_proxy_insertions_total, cache_proxy_misses_total,
    cache_proxy_rejections_total, note_proxy_hit, note_proxy_insertion, note_proxy_miss,
    note_proxy_rejection, reset_proxy_cache_metrics_for_tests,
};
pub use cache_serve::{serve_proxy_with_cache, PROXY_CACHE_NAMESPACE};
pub use errors::ProxyConfigError;
pub use forward::ForwardingHeaders;
pub use headers::{
    is_hop_by_hop_header, parse_response_content_length, request_headers_safe_for_proxy,
    strip_hop_by_hop_headers, ContentLengthParse, REQUEST_UPSTREAM_STRIP_HEADERS,
    RESPONSE_HOP_BY_HOP_HEADERS,
};
pub use hyper_client::{
    build_incoming_client, get_empty_body_client, hyper_client_config, set_hyper_client_config,
    HyperClientConfig, ProxyClient,
};
pub use hyper_forward::{
    bad_gateway, bad_request, forward_get_streaming, gateway_timeout, response_is_event_stream,
    service_unavailable, take_streaming, take_websocket, ProxyHyperMetrics,
};
pub use kernel_hooks::install_kernel_hooks;
pub use proxy_cache::{
    build_materialized_response, load_get_for_cache, prepare_proxy_cache_load, ProxyCacheLoad,
};
pub use runtime::{
    forward_websocket_by_cluster, global_hyper_metrics, load_get_for_cache_by_cluster, ProxyRuntime,
};
pub use selector::{
    endpoint_transport_identity, EndpointSelector, EndpointSpec, FailoverMode, SelectionOutcome,
};
pub use spike::run_spike_proxy;
pub use sse::{
    content_type_is_event_stream, path_is_sse_stream, upstream_timeout_for_path, SSE_STREAM_PATHS,
    SSE_STREAM_TIMEOUT,
};
pub use upstream::{UpstreamDescriptor, BENCH_SMALL_UPSTREAM_BODY};
pub use upstream_target::UpstreamTarget;
pub use uri::{build_uri, preseed_path_uris, PRESEED_PROXY_PATHS};
pub use websocket::{forward_websocket, is_websocket_upgrade};
pub use wire_stream::box_wire_stream;
#[cfg(unix)]
pub use wire_stream::box_wire_stream_with_fd;

/// Composition-root bundle for CLI registration (call core `register_proxy_dispatch_service(reg.service)`).
pub fn registration_with_default_runtime(
) -> exyonq_module_api::proxy_dispatch::ProxyRuntimeRegistration {
    use exyonq_module_api::proxy_dispatch::ProxyRuntimeRegistration;
    use std::sync::Arc;
    let runtime = Arc::new(ProxyRuntime::new());
    install_kernel_hooks(Arc::clone(&runtime));
    ProxyRuntimeRegistration {
        service: runtime,
        clusters: Arc::from([]),
    }
}
