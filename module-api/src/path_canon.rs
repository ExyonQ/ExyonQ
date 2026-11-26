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
//! H-01: canonical request path used by routing and every policy gate.
//!
//! The route that grants or denies access must see the same path as static,
//! proxy and WAF. This function percent-decodes once, removes dot segments,
//! collapses repeated separators and rejects ambiguous spellings (backslashes,
//! controls, malformed escapes, traversal above root).
//!
//! The result is the **URI form**, not a decoded display string: bytes that
//! cannot appear literally in an origin-form request-target are re-encoded. A
//! request-target is opaque bytes (nginx semantics), so `/a%20b` must keep
//! working and rebuild as a legal `http::Uri` instead of being rejected with
//! 400; consumers that need path *semantics* (the static filesystem resolver)
//! decode with [`decode_percent_path`] at the point where they touch the disk.
//! Keeping the canonical form self-consistent also makes the function
//! idempotent: `canonicalize_path(canonicalize_path(p)?)? == canonicalize_path(p)?`.

/// Canonicalize an HTTP request path. `None` means the request must be
/// rejected with 400 rather than interpreted by a downstream component.
pub fn canonicalize_path(path: &str) -> Option<String> {
    let decoded = decode_percent_path(path)?;
    let trailing_slash = decoded.last() == Some(&b'/');

    let mut segments: Vec<&[u8]> = Vec::new();
    for segment in decoded.split(|byte| *byte == b'/') {
        match segment {
            b"" | b"." => {}
            b".." => {
                // Above-root traversal is ambiguous across backends; fail closed.
                segments.pop()?;
            }
            _ => segments.push(segment),
        }
    }

    let mut canonical = String::with_capacity(decoded.len() + 1);
    canonical.push('/');
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            canonical.push('/');
        }
        encode_segment(segment, &mut canonical);
    }
    if trailing_slash && !segments.is_empty() {
        canonical.push('/');
    }
    Some(canonical)
}

/// Percent-decode a request path or canonical path into its raw bytes.
///
/// `None` for a malformed escape, a NUL/control byte or a backslash: those are
/// the spellings H-01 refuses instead of guessing what a backend would do.
/// Non-UTF-8 bytes are **not** rejected — a request-target is opaque bytes.
pub fn decode_percent_path(path: &str) -> Option<Vec<u8>> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = hex(bytes[i + 1])?;
            let lo = hex(bytes[i + 2])?;
            push_decoded((hi << 4) | lo, &mut decoded)?;
            i += 3;
        } else {
            push_decoded(bytes[i], &mut decoded)?;
            i += 1;
        }
    }
    Some(decoded)
}

fn push_decoded(byte: u8, out: &mut Vec<u8>) -> Option<()> {
    // NUL is already covered by `< 0x20`; the check is a single range on purpose.
    if byte < 0x20 || byte == 0x7f || byte == b'\\' {
        return None;
    }
    out.push(byte);
    Some(())
}

/// Percent-encode decoded text so it can be used as a URI path or `Location`.
///
/// Input is **decoded** text (a rewrite replacement, a capture): legal path
/// bytes stay literal and everything else — space, `%`, non-ASCII, controls —
/// is escaped with uppercase hex. nginx does the same when it turns a rewritten
/// `$uri` back into a request-target or a `Location` header.
pub fn escape_decoded_uri_bytes(text: &str) -> String {
    escape_uri_bytes(text.as_bytes())
}

/// Same encoding as [`escape_decoded_uri_bytes`], over raw decoded bytes.
///
/// A rewrite whose captures come from a decoded path that is not UTF-8 holds
/// bytes, not `str`; nginx re-encodes those bytes the same way it re-encodes
/// text, so both entry points share one encoder.
pub fn escape_uri_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        if is_literal_path_byte(byte) || matches!(byte, b'/' | b'?' | b'#') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(upper_hex(byte >> 4));
            out.push(upper_hex(byte & 0x0f));
        }
    }
    out
}

/// Append one decoded path segment in the form it must keep in a URI.
fn encode_segment(segment: &[u8], out: &mut String) {
    for &byte in segment {
        if is_literal_path_byte(byte) {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(upper_hex(byte >> 4));
            out.push(upper_hex(byte & 0x0f));
        }
    }
}

/// RFC 3986 `pchar` minus `pct-encoded`: unreserved / sub-delims / `:` / `@`.
fn is_literal_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'@'
        )
}

const fn upper_hex(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + (nibble - 10)) as char,
    }
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_path, decode_percent_path};

    #[test]
    fn collapses_dot_and_duplicate_segments() {
        for path in ["/./admin", "//admin", "/%2e/admin", "/a/../admin"] {
            assert_eq!(canonicalize_path(path).as_deref(), Some("/admin"), "{path}");
        }
    }

    #[test]
    fn decodes_escapes_once_and_keeps_the_uri_form() {
        assert_eq!(canonicalize_path("/%61dmin").as_deref(), Some("/admin"));
        assert_eq!(
            canonicalize_path("/admin%2fsecret").as_deref(),
            Some("/admin/secret")
        );
        // Non-ASCII stays percent-encoded: the canonical form is a URI, and it
        // must round-trip through a legal origin-form request-target.
        assert_eq!(
            canonicalize_path("/caf%C3%A9").as_deref(),
            Some("/caf%C3%A9")
        );
    }

    #[test]
    fn keeps_bytes_a_uri_cannot_carry_literally() {
        // `%20` is legal in a request-target and must not become a 400.
        assert_eq!(
            canonicalize_path("/%20index.html").as_deref(),
            Some("/%20index.html")
        );
        // `(` is a sub-delim: legal literally, so it is decoded to its literal form.
        assert_eq!(
            canonicalize_path("/fn%28%29.html").as_deref(),
            Some("/fn().html")
        );
        // Invalid UTF-8 is opaque bytes, not a parse error.
        assert_eq!(
            canonicalize_path("/i%FFndex.html").as_deref(),
            Some("/i%FFndex.html")
        );
        assert_eq!(
            canonicalize_path("/a%25b").as_deref(),
            Some("/a%25b"),
            "a literal percent must stay escaped"
        );
    }

    #[test]
    fn canonical_form_is_idempotent() {
        for path in [
            "/%20index.html",
            "/fn%28%29.html",
            "/i%FFndex.html",
            "/caf%C3%A9",
            "/a%25b",
            "/a/b/",
        ] {
            let once = canonicalize_path(path).expect(path);
            assert_eq!(
                canonicalize_path(&once).as_deref(),
                Some(once.as_str()),
                "{path}"
            );
        }
    }

    #[test]
    fn decode_restores_filesystem_bytes() {
        assert_eq!(
            decode_percent_path("/hello%20world.html").as_deref(),
            Some(&b"/hello world.html"[..])
        );
        assert_eq!(
            decode_percent_path("/i%FFndex.html").as_deref(),
            Some(&b"/i\xFFndex.html"[..])
        );
    }

    #[test]
    fn double_encoding_stays_literal() {
        // One decode leaves `%2e` as text, so the second encoding is inert: the
        // canonical form keeps it escaped and never becomes a dot segment.
        assert_eq!(
            canonicalize_path("/%252e%252e/x").as_deref(),
            Some("/%252e%252e/x")
        );
        // The same escape, once decoded, is a dot segment and fails closed.
        for path in ["/.%2e/x", "/%2e./x"] {
            assert!(canonicalize_path(path).is_none(), "{path}");
        }
    }

    #[test]
    fn rejects_ambiguous_or_unsafe_spellings() {
        for path in [
            "/../admin",
            "/%2e%2e/admin",
            "/admin%5csecret",
            "/%zz",
            "/%00",
            "/bad%0dname",
            "/a\u{0}b",
            "/a\u{7f}b",
        ] {
            assert!(canonicalize_path(path).is_none(), "{path}");
        }
    }

    #[test]
    fn preserves_trailing_directory_slash() {
        assert_eq!(canonicalize_path("/admin/").as_deref(), Some("/admin/"));
        assert_eq!(canonicalize_path("/").as_deref(), Some("/"));
        assert_eq!(canonicalize_path("//").as_deref(), Some("/"));
        assert_eq!(
            canonicalize_path("").as_deref(),
            Some("/"),
            "an empty target normalizes like a bare slash"
        );
    }
}
