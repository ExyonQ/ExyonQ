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
//! Bounded in-memory FastCGI record parser — no network I/O.

use crate::record::{RecordHeader, FCGI_VERSION_1, RECORD_HEADER_LEN};

/// Fail-closed parse errors for hostile or truncated input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    BufferTooShort { need: usize, have: usize },
    UnsupportedVersion(u8),
    TruncatedContent { expected: usize, have: usize },
    TruncatedPadding { expected: usize, have: usize },
}

/// One fully parsed FastCGI record view into the caller buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedRecord<'a> {
    pub header: RecordHeader,
    pub content: &'a [u8],
    pub padding: &'a [u8],
    pub consumed: usize,
}

/// Parse the 8-byte header from `buf`. Does not validate content/padding presence.
pub fn parse_header(buf: &[u8]) -> Result<RecordHeader, ParseError> {
    if buf.len() < RECORD_HEADER_LEN {
        return Err(ParseError::BufferTooShort {
            need: RECORD_HEADER_LEN,
            have: buf.len(),
        });
    }

    let version = buf[0];
    if version != FCGI_VERSION_1 {
        return Err(ParseError::UnsupportedVersion(version));
    }

    Ok(RecordHeader {
        version,
        record_type: buf[1],
        request_id: u16::from_be_bytes([buf[2], buf[3]]),
        content_length: u16::from_be_bytes([buf[4], buf[5]]),
        padding_length: buf[6],
        reserved: buf[7],
    })
}

/// Parse one complete record (header + content + padding) from the start of `buf`.
pub fn parse_record(buf: &[u8]) -> Result<ParsedRecord<'_>, ParseError> {
    let header = parse_header(buf)?;
    let content_len = usize::from(header.content_length);
    let padding_len = usize::from(header.padding_length);
    let after_header = RECORD_HEADER_LEN;

    let content_end = after_header + content_len;

    if buf.len() < content_end {
        return Err(ParseError::TruncatedContent {
            expected: content_len,
            have: buf.len().saturating_sub(after_header),
        });
    }

    let padding_end = content_end + padding_len;

    if buf.len() < padding_end {
        return Err(ParseError::TruncatedPadding {
            expected: padding_len,
            have: buf.len().saturating_sub(content_end),
        });
    }

    Ok(ParsedRecord {
        header,
        content: &buf[after_header..content_end],
        padding: &buf[content_end..padding_end],
        consumed: padding_end,
    })
}
