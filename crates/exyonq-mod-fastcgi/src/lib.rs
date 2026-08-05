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
//! Plan 08 FastCGI module boundary — PR4-A mock roundtrip; PR5-A-min wire module-only.
//!
//! Core hot path returns **501** for `Backend::Fastcgi` until PR5-A2+ authorization.

pub mod adapter;
pub mod bridge;
pub mod cache_metrics;
pub mod cache_serve;
pub mod caps;
pub mod client;
pub mod commit;
#[cfg(unix)]
pub mod conn_pool;
pub mod encode;
#[cfg(unix)]
pub mod fcgi_stream;
pub mod metrics;
pub mod mock;
pub mod params;
pub mod parser;
pub mod pool;
#[cfg(unix)]
pub mod pooled_forward;
pub mod record;
pub mod request_abort;
pub mod runtime;
pub mod script_resolver;
pub mod timeout_budget;
pub mod transport;
#[cfg(unix)]
pub mod unix_connect;
pub mod unix_transport;
pub mod wire;

pub use adapter::{
    is_tcp_pool_address, is_unix_socket_path, map_client_error, map_forward_success,
    map_transport_error, parse_cgi_stdout, parse_tcp_pool_address, resolve_pool_endpoints,
    resolve_unix_socket_pools, FcgiModuleExecutor, MockFcgiExecutor,
};
#[cfg(feature = "core-bridge")]
pub use bridge::register_with_core;
pub use cache_metrics::{
    cache_fcgi_hits_total, cache_fcgi_insertions_total, cache_fcgi_misses_total,
    cache_fcgi_rejections_total, reset_fcgi_cache_metrics_for_tests,
};
pub use cache_serve::{
    prepare_fcgi_cache_load, serve_fastcgi_with_cache, serve_fastcgi_with_cache_hook,
    FcgiCacheLoad, FCGI_CACHE_NAMESPACE,
};
pub use caps::{
    CapError, FCGI_CONNECT_TIMEOUT, FCGI_READ_TIMEOUT, FCGI_WRITE_TIMEOUT, MAX_CGI_HEADER_BYTES,
    MAX_CGI_HEADER_COUNT, MAX_FCGI_PARAMS_BYTES, MAX_FCGI_RECORDS, MAX_FCGI_RESPONSE_BYTES,
    MAX_FCGI_STDERR_BYTES, MAX_FCGI_STDIN_BYTES,
};
pub use client::{ClientError, ForwardResponse, PhpFpmClient, PoolLabel};
pub use commit::CommitStage;
#[cfg(unix)]
pub use conn_pool::{
    next_pool_generation, ConnPool, ConnPoolConfig, ConnPoolSet, ConnPoolStats,
    DEFAULT_FCGI_IDLE_TIMEOUT,
};
pub use encode::{
    append_length, decode_forward_response, decode_forward_response_with_stderr,
    encode_begin_request_frame, encode_begin_request_frame_with_flags, encode_params_body,
    encode_params_body_owned, encode_params_frames, encode_params_frames_owned,
    encode_record_frame, encode_stdin_frames, DecodeError, DecodedResponse, EncodeError,
};
pub use exyonq_module_api::fcgi_dispatch::{
    FcgiBindError, FcgiCompiledSlot, FcgiRuntimeRegistration,
};
#[cfg(unix)]
pub use fcgi_stream::{FcgiStream, PoolEndpoint};
#[cfg(test)]
pub use metrics::fcgi_metric_test_gate;
pub use metrics::{
    fcgi_inflight_current, fcgi_recovery_seconds, fcgi_responses_200_total,
    fcgi_responses_501_success_not_authorized_total, fcgi_responses_501_total,
    fcgi_responses_502_total, fcgi_responses_503_total, fcgi_responses_504_total,
    fcgi_saturation_rejections_total, pool_ops_snapshot, snapshot as fcgi_metrics_snapshot,
    FcgiPoolOpsSnapshot,
};
pub use mock::{MockFpmConfig, MockFpmTransport, PR5B1_BODY, PR5B1_STDOUT};
pub use params::{http_header_to_cgi_param, MinForwardRequest, ParamsError, DEFAULT_REMOTE_ADDR};
pub use parser::{parse_header, parse_record, ParseError, ParsedRecord};
pub use pool::{
    resolve_pool_capacities, resolve_pool_capacities_with_transport, validate_max_concurrency,
    DEFAULT_FCGI_MAX_CONCURRENCY, MAX_FCGI_MAX_CONCURRENCY,
};
pub use record::{
    RecordHeader, END_REQUEST_BODY_LEN, FCGI_ABORT_REQUEST, FCGI_BEGIN_REQUEST, FCGI_END_REQUEST,
    FCGI_KEEP_CONN, FCGI_PARAMS, FCGI_REQUEST_COMPLETE, FCGI_STDERR, FCGI_STDIN, FCGI_STDOUT,
    FCGI_VERSION_1, MAX_CONTENT_LENGTH, MAX_PADDING_LENGTH, MAX_RECORD_FRAME_LEN,
    RECORD_HEADER_LEN,
};
pub use runtime::{FcgiRuntime, FCGI_DELEGATE_TIMEOUT};
pub use script_resolver::FastcgiScriptResolver;
pub use timeout_budget::{
    TimeoutBudget, TimeoutPhase, DEFAULT_FCGI_CHECKOUT_TIMEOUT, DEFAULT_FCGI_TOTAL_TIMEOUT,
};
pub use transport::{
    FastcgiRecordTransport, InertTransport, TransportError, ValidatingMockTransport,
};
#[cfg(unix)]
pub use unix_connect::connect_unix_stream;
pub use unix_transport::UnixFpmTransport;
pub use wire::{WireEndpoint, WireError, WireTransport};

/// Stable module identifier for composition registration (future PR-7).
pub const MODULE_NAME: &str = "exyonq-mod-fastcgi";

/// PR3-A phase marker — parser + inert transport + client skeleton.
pub const PR3A_PHASE: &str = "parser-transport-client-skeleton";

/// PR4-A phase marker — in-memory mock FPM roundtrip; no wire I/O.
pub const PR4A_PHASE: &str = "mock-fpm-roundtrip-in-memory";

/// PR5-A-min phase marker — real wire transport module-only; live core still **501**.
pub const PR5A_MIN_PHASE: &str = "wire-fpm-module-only";
