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
//! Sync FastCGI wire — codec, PARAMS, commit/retry, exchange.
//!
//! ```text
//! TOKIO = 0
//! HYPER = 0
//! ```
//!
//! Portable logic adapted from `exyonq-mod-fastcgi` without Tokio/Hyper/module-api.

mod caps;
mod cgi;
mod commit;
mod encode;
mod exchange;
mod params;
mod parser;
mod record;

pub use caps::{
    check_add_cap, check_size_cap, CapError, FCGI_CONNECT_TIMEOUT, FCGI_READ_TIMEOUT,
    FCGI_WRITE_TIMEOUT, MAX_CGI_HEADER_BYTES, MAX_CGI_HEADER_COUNT, MAX_FCGI_PARAMS_BYTES,
    MAX_FCGI_RECORDS, MAX_FCGI_RESPONSE_BYTES, MAX_FCGI_STDERR_BYTES, MAX_FCGI_STDIN_BYTES,
};
pub use cgi::{parse_cgi_stdout, CgiParseError, CgiResponse};
pub use commit::CommitStage;
pub use encode::{
    append_length, decode_forward_response, decode_forward_response_with_stderr,
    encode_begin_request_frame, encode_begin_request_frame_with_flags, encode_header,
    encode_params_body, encode_params_body_owned, encode_params_frames, encode_params_frames_owned,
    encode_record_frame, encode_stdin_frames, DecodeError, DecodedResponse, EncodeError,
};
pub use exchange::{
    exchange_once, exchange_once_with_not_started_retry, forward_on_stream, ExchangeAttempt,
    ExchangeError, ExchangeResponse,
};
pub use params::{http_header_to_cgi_param, MinForwardRequest, ParamsError, DEFAULT_REMOTE_ADDR};
pub use parser::{parse_header, parse_record, ParseError, ParsedRecord};
pub use record::{
    RecordHeader, END_REQUEST_BODY_LEN, FCGI_ABORT_REQUEST, FCGI_BEGIN_REQUEST, FCGI_END_REQUEST,
    FCGI_KEEP_CONN, FCGI_PARAMS, FCGI_REQUEST_COMPLETE, FCGI_STDERR, FCGI_STDIN, FCGI_STDOUT,
    FCGI_VERSION_1, MAX_CONTENT_LENGTH, MAX_PADDING_LENGTH, MAX_RECORD_FRAME_LEN,
    RECORD_HEADER_LEN,
};
