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
//! FastCGI v1 record header types — sync wire, in-memory only.

/// FastCGI record header size in bytes.
pub const RECORD_HEADER_LEN: usize = 8;

/// Supported FastCGI protocol version (v1).
pub const FCGI_VERSION_1: u8 = 1;

/// Maximum `content_length` per FastCGI record (spec limit).
pub const MAX_CONTENT_LENGTH: u16 = u16::MAX;

/// Maximum `padding_length` per FastCGI record (single byte).
pub const MAX_PADDING_LENGTH: u8 = u8::MAX;

/// Maximum bytes for one complete FastCGI v1 record (header + content + padding).
pub const MAX_RECORD_FRAME_LEN: usize =
    RECORD_HEADER_LEN + MAX_CONTENT_LENGTH as usize + MAX_PADDING_LENGTH as usize;

/// FastCGI record types (v1).
pub const FCGI_BEGIN_REQUEST: u8 = 1;
pub const FCGI_ABORT_REQUEST: u8 = 2;
pub const FCGI_END_REQUEST: u8 = 3;
pub const FCGI_PARAMS: u8 = 4;
pub const FCGI_STDIN: u8 = 5;
pub const FCGI_STDOUT: u8 = 6;
pub const FCGI_STDERR: u8 = 7;

/// `END_REQUEST` protocol status — request complete.
pub const FCGI_REQUEST_COMPLETE: u8 = 0;

/// `FCGI_BEGIN_REQUEST` flags — keep connection after request (pooling).
pub const FCGI_KEEP_CONN: u8 = 1;

/// `END_REQUEST` record body length in bytes.
pub const END_REQUEST_BODY_LEN: usize = 8;

/// Parsed 8-byte FastCGI record header (big-endian multi-byte fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordHeader {
    pub version: u8,
    pub record_type: u8,
    pub request_id: u16,
    pub content_length: u16,
    pub padding_length: u8,
    pub reserved: u8,
}

impl RecordHeader {
    /// Total bytes required to hold header + content + padding.
    pub fn frame_len(&self) -> usize {
        RECORD_HEADER_LEN + usize::from(self.content_length) + usize::from(self.padding_length)
    }
}
