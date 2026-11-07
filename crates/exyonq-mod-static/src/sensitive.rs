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
//! H-01: static-file sensitivity policy (default deny).
//!
//! The policy is deliberately evaluated on a decoded copy of the request
//! path and on the canonical root-relative target. Resolution still uses the
//! original spelling, so this cannot turn an encoded name into a different
//! filesystem path; it only classifies it as forbidden before I/O.
//!
//! Dotfiles are denied except the ACME `.well-known` directory when it is the
//! first path component. Server-side executable extensions are denied
//! case-insensitively. A malformed percent escape fails closed.

use crate::StaticError;
use std::path::Path;

const WELL_KNOWN: &[u8] = b".well-known";

/// Extensions that a static server must never expose as source/executables.
/// Keep this list explicit: a suffix scan is easier to review than a regex.
const DENIED_EXTENSIONS: &[&[u8]] = &[
    b"php", b"php3", b"php4", b"php5", b"php7", b"php8", b"phps", b"phtml", b"phar", b"inc",
    b"cgi", b"pl", b"py", b"rb", b"sh", b"bash", b"zsh", b"fish", b"asp", b"aspx", b"ascx",
    b"ashx", b"asmx", b"jsp", b"jspx", b"jspf", b"do", b"action", b"cfm", b"cfml", b"shtml",
    b"shtm", b"hta",
];

/// Validate a root-relative path (the part after the route prefix).
///
/// `PathTraversal` is returned for `..`; `SensitivePath` for a denied name.
/// `SensitivePath` becomes an opaque 404 at the transport boundary; the
/// pre-existing traversal policy remains 403 and does not disclose a file.
pub(crate) fn ensure_relative_allowed(relative: &str) -> Result<(), StaticError> {
    let decoded = decode_percent_once(relative.as_bytes()).ok_or(StaticError::SensitivePath)?;
    check_decoded(&decoded)
}

/// Defence in depth for direct file-serving helpers that only receive a
/// filesystem path. Only executable extensions are classified here: these
/// helpers also serve internal temporary files (for example `.tmp…`) and do
/// not have the root-relative context needed to distinguish them from a
/// request dotfile. The authoritative dotfile policy runs in
/// [`ensure_relative_allowed`] before any request reaches these helpers.
pub(crate) fn ensure_file_name_allowed(path: &Path) -> Result<(), StaticError> {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Err(StaticError::SensitivePath);
    };
    let decoded = decode_percent_once(name.as_bytes()).ok_or(StaticError::SensitivePath)?;
    if decoded.is_empty() {
        return Err(StaticError::SensitivePath);
    }
    if let Some(ext) = extension(&decoded) {
        if is_denied_extension(ext) {
            return Err(StaticError::SensitivePath);
        }
    }
    Ok(())
}

fn check_decoded(decoded: &[u8]) -> Result<(), StaticError> {
    let mut normal_index = 0usize;
    for segment in decoded.split(|b| *b == b'/' || *b == b'\\') {
        if segment.is_empty() || segment == b"." {
            continue;
        }
        if segment == b".." {
            return Err(StaticError::PathTraversal);
        }
        if segment[0] == b'.' {
            if normal_index == 0 && segment == WELL_KNOWN {
                normal_index += 1;
                continue;
            }
            return Err(StaticError::SensitivePath);
        }
        if let Some(ext) = extension(segment) {
            if is_denied_extension(ext) {
                return Err(StaticError::SensitivePath);
            }
        }
        normal_index += 1;
    }
    Ok(())
}

fn extension(segment: &[u8]) -> Option<&[u8]> {
    let dot = segment.iter().rposition(|b| *b == b'.')?;
    if dot == 0 || dot + 1 >= segment.len() {
        return None;
    }
    Some(&segment[dot + 1..])
}

fn is_denied_extension(ext: &[u8]) -> bool {
    DENIED_EXTENSIONS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(ext))
}

/// One decode only, matching the HTTP parser's path spelling for this policy.
/// Invalid escapes fail closed rather than falling through to a literal name.
fn decode_percent_once(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0usize;
    while i < input.len() {
        if input[i] != b'%' {
            out.push(input[i]);
            i += 1;
            continue;
        }
        if i + 2 >= input.len() {
            return None;
        }
        let hi = hex(input[i + 1])?;
        let lo = hex(input[i + 2])?;
        out.push((hi << 4) | lo);
        i += 3;
    }
    Some(out)
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
    use super::*;

    fn allowed(path: &str) -> bool {
        ensure_relative_allowed(path).is_ok()
    }

    #[test]
    fn denies_dotfiles_and_server_side_extensions() {
        for path in [
            ".env",
            ".git/config",
            "config.php",
            "CONFIG.PHP",
            "index.phtml",
            "archive.phar",
            "legacy.php3",
            "legacy.php4",
            "legacy.php5",
            "legacy.php7",
            "legacy.php8",
            "source.phps",
            "settings.inc",
            "cgi-bin/x.cgi",
            "script.py",
            "shell.sh",
            "page.aspx",
            "app.jsp",
        ] {
            assert!(!allowed(path), "expected deny: {path}");
        }
    }

    #[test]
    fn allows_ordinary_assets_and_well_known() {
        for path in [
            "ok.txt",
            "assets/app.js",
            "assets/style.css",
            "archive.tar.gz",
            ".well-known",
            ".well-known/test",
            ".well-known/acme-challenge/token",
        ] {
            assert!(allowed(path), "expected allow: {path}");
        }
    }

    #[test]
    fn well_known_is_not_a_traversal_or_nested_dotfile_escape() {
        assert!(matches!(
            ensure_relative_allowed(".well-known/../.env"),
            Err(StaticError::PathTraversal)
        ));
        assert!(!allowed(".well-known/.env"));
        assert!(!allowed("nested/.well-known/test"));
    }

    #[test]
    fn percent_and_case_variants_are_denied() {
        for path in [
            "%2eenv",
            "%2EENV",
            "%2e%2e/ok.txt",
            "config%2Ephp",
            "%2e%77ell-known/.env",
            "..%2f.env",
            "ok%zz.txt",
        ] {
            assert!(!allowed(path), "expected deny: {path}");
        }
        assert!(allowed("%2e%77ell-known/test"));
    }

    #[test]
    fn file_name_guard_denies_direct_sensitive_file() {
        assert!(matches!(
            ensure_file_name_allowed(Path::new("/site/config.php")),
            Err(StaticError::SensitivePath)
        ));
        assert!(matches!(
            ensure_file_name_allowed(Path::new("/site/shell.PHTML")),
            Err(StaticError::SensitivePath)
        ));
        assert!(ensure_file_name_allowed(Path::new("/site/ok.txt")).is_ok());
        assert!(ensure_file_name_allowed(Path::new("/site/.well-known/test")).is_ok());
        assert!(ensure_file_name_allowed(Path::new("/site/.tmp-internal")).is_ok());
    }
}
