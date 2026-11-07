//! Canonical extension → `Content-Type` table.
//!
//! Single source of truth for every path that emits a static representation
//! (mod-static wire/sendfile, CFD dataplane static_serve, future generators).
//! A missing entry falls back to `application/octet-stream` — browsers then
//! refuse to use the asset (`type="module"` JS, fonts, images), so new web
//! formats must be added here, not in per-crate tables (EXY-02/EXY-09).

use std::path::Path;

/// Map a lowercase file extension (no dot) to a `Content-Type` value.
pub fn content_type_for_ext(ext: &str) -> &'static str {
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        // RFC 9239: text/javascript is the canonical JS media type.
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        "pdf" => "application/pdf",
        "wasm" => "application/wasm",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Map a path's extension (case-insensitive) to a `Content-Type` value.
pub fn content_type_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some(ext) => content_type_for_ext(ext),
        None => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every extension any serving path must recognize (EXY-02 regression:
    /// 0.8.0 shipped octet-stream for webp/avif/ico/xml/woff/wasm/pdf/gif/htm/mjs).
    #[test]
    fn canonical_table_covers_web_assets() {
        let cases: &[(&str, &str)] = &[
            ("html", "text/html; charset=utf-8"),
            ("htm", "text/html; charset=utf-8"),
            ("css", "text/css; charset=utf-8"),
            ("js", "text/javascript; charset=utf-8"),
            ("mjs", "text/javascript; charset=utf-8"),
            ("json", "application/json; charset=utf-8"),
            ("txt", "text/plain; charset=utf-8"),
            ("xml", "application/xml; charset=utf-8"),
            ("webmanifest", "application/manifest+json"),
            ("pdf", "application/pdf"),
            ("wasm", "application/wasm"),
            ("png", "image/png"),
            ("jpg", "image/jpeg"),
            ("jpeg", "image/jpeg"),
            ("gif", "image/gif"),
            ("webp", "image/webp"),
            ("avif", "image/avif"),
            ("svg", "image/svg+xml"),
            ("ico", "image/x-icon"),
            ("woff", "font/woff"),
            ("woff2", "font/woff2"),
            ("bin", "application/octet-stream"),
        ];
        for (ext, want) in cases {
            assert_eq!(content_type_for_ext(ext), *want, "extension {ext}");
        }
    }

    #[test]
    fn path_lookup_is_case_insensitive_and_missing_safe() {
        assert_eq!(content_type_for_path(Path::new("LOGO.AVIF")), "image/avif");
        assert_eq!(
            content_type_for_path(Path::new("no_ext")),
            "application/octet-stream"
        );
    }
}
