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
//! CRS `FILES` / `FILES_NAMES` / `FILES_COMBINED_SIZE` — bounded
//! `multipart/form-data` inspection.
//!
//! ModSecurity's `FILES` collection holds the original uploaded file names,
//! `FILES_NAMES` the form field names that carried a file, and
//! `FILES_COMBINED_SIZE` the sum of the file payload sizes. Only part headers
//! and sizes are materialized; file bytes are measured, never copied.

/// One multipart part that carried a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilePart {
    /// `Content-Disposition` field name.
    pub field: String,
    /// Original `filename` from `Content-Disposition`.
    pub filename: String,
    /// Payload length in bytes.
    pub size: usize,
}

const MAX_PARTS: usize = 256;
const MAX_PART_HEADER_BYTES: usize = 8 * 1024;

/// Parse the file parts of a `multipart/form-data` body.
///
/// Returns `None` when the content type is not multipart or the boundary is
/// missing/malformed. Malformed individual parts are skipped.
pub(crate) fn file_parts(content_type: &str, body: &[u8]) -> Option<Vec<FilePart>> {
    let lower = content_type.to_ascii_lowercase();
    if !lower.starts_with("multipart/form-data") {
        return None;
    }
    let boundary = boundary_of(content_type)?;
    let delimiter = format!("--{boundary}").into_bytes();
    let mut parts = Vec::new();
    let mut rest = body;
    while let Some(pos) = find(rest, &delimiter) {
        rest = &rest[pos + delimiter.len()..];
        if rest.starts_with(b"--") {
            break;
        }
        rest = rest.strip_prefix(b"\r\n").unwrap_or(rest);
        let Some(head_end) = find(rest, b"\r\n\r\n") else {
            break;
        };
        if head_end > MAX_PART_HEADER_BYTES {
            break;
        }
        let head = &rest[..head_end];
        let payload = &rest[head_end + 4..];
        let next = find(payload, &delimiter).unwrap_or(payload.len());
        // The CRLF before the next boundary belongs to the delimiter.
        let size = next.saturating_sub(2);
        if let Some(part) = parse_part_header(head, size) {
            parts.push(part);
            if parts.len() >= MAX_PARTS {
                break;
            }
        }
        rest = &payload[next..];
    }
    Some(parts)
}

/// Every raw header line of every multipart part, in order. CRS exposes these
/// as `MULTIPART_PART_HEADERS` (922140/922150/922110).
pub(crate) fn part_header_lines(content_type: &str, body: &[u8]) -> Vec<String> {
    let lower = content_type.to_ascii_lowercase();
    if !lower.starts_with("multipart/form-data") {
        return Vec::new();
    }
    let Some(boundary) = boundary_of(content_type) else {
        return Vec::new();
    };
    let delimiter = format!("--{boundary}").into_bytes();
    let mut headers = Vec::new();
    let mut rest = body;
    while let Some(pos) = find(rest, &delimiter) {
        rest = &rest[pos + delimiter.len()..];
        if rest.starts_with(b"--") {
            break;
        }
        rest = rest.strip_prefix(b"\r\n").unwrap_or(rest);
        let Some(head_end) = find(rest, b"\r\n\r\n") else {
            break;
        };
        if head_end > MAX_PART_HEADER_BYTES {
            break;
        }
        let head = &rest[..head_end];
        for line in String::from_utf8_lossy(head).split("\r\n") {
            let line = line.trim_end_matches('\r');
            if !line.is_empty() {
                headers.push(line.to_string());
                if headers.len() >= MAX_PARTS * 8 {
                    return headers;
                }
            }
        }
        let payload = &rest[head_end + 4..];
        let next = find(payload, &delimiter).unwrap_or(payload.len());
        rest = &payload[next..];
    }
    headers
}

/// ModSecurity's `MULTIPART_STRICT_ERROR` check (modsecurity.conf rule
/// 200003): a multipart body that fails strict validation is rejected with
/// 400. The regression corpus intentionally carries LF-only bodies, exactly
/// the `LF` flag ModSecurity reports, so parity requires the same rejection.
pub(crate) fn strict_error(content_type: &str, body: &[u8]) -> Option<&'static str> {
    let lower = content_type.to_ascii_lowercase();
    if !lower.starts_with("multipart/form-data") {
        return None;
    }
    let Some(boundary) = boundary_of(content_type) else {
        return Some("multipart body without a valid boundary");
    };
    let delimiter = format!("--{boundary}").into_bytes();
    let next_delimiter = [b"\r\n".as_slice(), delimiter.as_slice()].concat();
    let mut rest = body;
    while let Some(pos) = find(rest, &delimiter) {
        rest = &rest[pos + delimiter.len()..];
        if let Some(tail) = rest.strip_prefix(b"--") {
            if tail.starts_with(b"\n") {
                return Some("multipart body has a bare LF line ending");
            }
            if tail.starts_with(b"\r") && !tail.starts_with(b"\r\n") {
                return Some("multipart body has a bare CR line ending");
            }
            return None;
        }
        if !rest.starts_with(b"\r\n") {
            return Some(if rest.starts_with(b"\n") {
                "multipart body has a bare LF line ending"
            } else if rest.starts_with(b"\r") {
                "multipart body has a bare CR line ending"
            } else {
                "multipart boundary is not followed by CRLF"
            });
        }
        rest = &rest[2..];
        let Some(head_end) = find(rest, b"\r\n\r\n") else {
            return Some("multipart part headers are not CRLF terminated");
        };
        if let Some(reason) = bare_line_ending(&rest[..head_end]) {
            return Some(reason);
        }
        let payload = &rest[head_end + 4..];
        let next = find(payload, &next_delimiter)?;
        rest = &payload[next + 2..];
    }
    None
}

fn bare_line_ending(bytes: &[u8]) -> Option<&'static str> {
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' && (index == 0 || bytes[index - 1] != b'\r') {
            return Some("multipart body has a bare LF line ending");
        }
        if *byte == b'\r' && bytes.get(index + 1) != Some(&b'\n') {
            return Some("multipart body has a bare CR line ending");
        }
    }
    None
}

fn parse_part_header(head: &[u8], size: usize) -> Option<FilePart> {
    let text = String::from_utf8_lossy(head);
    let mut field = None;
    let mut filename = None;
    for line in text.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("content-disposition") {
            continue;
        }
        for param in value.split(';').skip(1) {
            let Some((key, raw)) = param.split_once('=') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let raw = raw.trim().trim_matches('"');
            match key.as_str() {
                "name" => field = Some(raw.to_string()),
                "filename" => filename = Some(raw.to_string()),
                _ => {}
            }
        }
    }
    Some(FilePart {
        field: field?,
        filename: filename?,
        size,
    })
}

fn boundary_of(content_type: &str) -> Option<String> {
    for param in content_type.split(';').skip(1) {
        let Some((key, value)) = param.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("boundary") {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() && value.len() <= 70 {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(boundary: &str, parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, filename, content) in parts {
            out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            let disposition = match filename {
                Some(filename) => format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n"
                ),
                None => format!("Content-Disposition: form-data; name=\"{name}\"\r\n"),
            };
            out.extend_from_slice(disposition.as_bytes());
            out.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
            out.extend_from_slice(content);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        out
    }

    #[test]
    fn extracts_file_names_fields_and_sizes() {
        let raw = body(
            "XyZ",
            &[
                ("note", None, b"hello"),
                ("upload", Some("shell.php"), b"<?php echo 1; ?>"),
                ("upload2", Some("a.bin"), &[0u8, 1, 2]),
            ],
        );
        let parts = file_parts("multipart/form-data; boundary=XyZ", &raw).expect("multipart");
        assert_eq!(
            parts,
            vec![
                FilePart {
                    field: "upload".into(),
                    filename: "shell.php".into(),
                    size: 16
                },
                FilePart {
                    field: "upload2".into(),
                    filename: "a.bin".into(),
                    size: 3
                },
            ]
        );
    }

    #[test]
    fn quoted_boundary_and_no_files() {
        let raw = body("abc", &[("field", None, b"v")]);
        assert_eq!(
            file_parts("multipart/form-data; boundary=\"abc\"", &raw),
            Some(Vec::new())
        );
    }

    #[test]
    fn non_multipart_and_missing_boundary_are_none() {
        assert!(file_parts("application/x-www-form-urlencoded", b"a=b").is_none());
        assert!(file_parts("multipart/form-data", b"--x--").is_none());
    }

    #[test]
    fn strict_multipart_rejects_bare_lf_and_accepts_crlf() {
        let lf = b"--b\nContent-Disposition: form-data; name=\"f\"\n\nv\n--b--\n";
        assert!(strict_error("multipart/form-data; boundary=b", lf).is_some());
        let crlf = b"--b\r\nContent-Disposition: form-data; name=\"f\"\r\n\r\nv\r\n--b--\r\n";
        assert!(strict_error("multipart/form-data; boundary=b", crlf).is_none());
    }

    #[test]
    fn strict_multipart_validates_closing_delimiter_line_ending() {
        assert!(strict_error("multipart/form-data; boundary=b", b"--b--\n").is_some());
        assert!(strict_error("multipart/form-data; boundary=b", b"--b--\r\n").is_none());
    }

    #[test]
    fn strict_multipart_accepts_binary_payload_line_bytes() {
        let payload: Vec<u8> = (0..=255).collect();
        let raw = body("XyZ", &[("blob", Some("data.bin"), &payload)]);
        assert!(strict_error("multipart/form-data; boundary=XyZ", &raw).is_none());
    }

    #[test]
    fn truncated_body_does_not_panic() {
        let raw = b"--b\r\nContent-Disposition: form-data; name=\"f\"; filename=\"x\"\r\n";
        assert_eq!(
            file_parts("multipart/form-data; boundary=b", raw),
            Some(Vec::new())
        );
    }
}
