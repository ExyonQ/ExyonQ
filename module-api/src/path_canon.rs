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
//! proxy and WAF. This function decodes percent escapes once, removes dot
//! segments, collapses repeated separators and rejects ambiguous spellings
//! (backslashes, controls, malformed escapes, traversal above root).

/// Canonicalize an HTTP request path. `None` means the request must be
/// rejected with 400 rather than interpreted by a downstream component.
pub fn canonicalize_path(path: &str) -> Option<String> {
    if path.is_empty() {
        return Some("/".to_string());
    }

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

    let decoded = String::from_utf8(decoded).ok()?;
    let trailing_slash = decoded.ends_with('/');
    let mut segments: Vec<&str> = Vec::new();
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                // Above-root traversal is ambiguous across backends; fail closed.
                segments.pop()?;
            }
            _ => segments.push(segment),
        }
    }

    let mut canonical = String::with_capacity(decoded.len() + 1);
    canonical.push('/');
    canonical.push_str(&segments.join("/"));
    if trailing_slash && !segments.is_empty() {
        canonical.push('/');
    }
    Some(canonical)
}

fn push_decoded(byte: u8, out: &mut Vec<u8>) -> Option<()> {
    if byte == 0 || byte < 0x20 || byte == 0x7f || byte == b'\\' {
        return None;
    }
    match byte {
        // Keep the decoded spelling unambiguous if the path is rebuilt as a URI.
        b'%' => out.extend_from_slice(b"%25"),
        b'?' => out.extend_from_slice(b"%3F"),
        b'#' => out.extend_from_slice(b"%23"),
        _ => out.push(byte),
    }
    Some(())
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
    use super::canonicalize_path;

    #[test]
    fn collapses_dot_and_duplicate_segments() {
        for path in ["/./admin", "//admin", "/%2e/admin", "/a/../admin"] {
            assert_eq!(canonicalize_path(path).as_deref(), Some("/admin"), "{path}");
        }
    }

    #[test]
    fn decodes_unreserved_bytes_once() {
        assert_eq!(canonicalize_path("/%61dmin").as_deref(), Some("/admin"));
        assert_eq!(
            canonicalize_path("/admin%2fsecret").as_deref(),
            Some("/admin/secret")
        );
        assert_eq!(canonicalize_path("/caf%C3%A9").as_deref(), Some("/café"));
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
        ] {
            assert!(canonicalize_path(path).is_none(), "{path}");
        }
    }

    #[test]
    fn preserves_trailing_directory_slash() {
        assert_eq!(canonicalize_path("/admin/").as_deref(), Some("/admin/"));
        assert_eq!(canonicalize_path("/").as_deref(), Some("/"));
        assert_eq!(canonicalize_path("//").as_deref(), Some("/"));
    }
}
