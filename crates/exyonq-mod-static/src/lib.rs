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
//! KD2 static file module — path resolution, preload, precooked/wire/sendfile assets.

#[cfg(test)]
mod wire_eligibility_tests;

mod body;
mod bounded_read;
pub mod cache_serve;
#[cfg(target_os = "linux")]
mod fd_io;
pub mod identity;
mod precooked;
mod root;
pub mod sendfile;
pub mod static_cache;
pub mod wire;

#[cfg(target_os = "linux")]
mod epoll_bench;
#[cfg(target_os = "linux")]
mod epoll_session;
mod kernel_hooks;
mod outcome;
pub mod runtime;
#[cfg(target_os = "linux")]
pub mod sendfile_fsm;
mod sendfile_handle;
#[cfg(target_os = "linux")]
pub mod sendfile_metrics;
pub mod wire_conn;
pub mod wire_eligibility;
mod wire_io;
pub use cache_serve::{
    invalidate_static_path, serve_static_with_cache, StaticCacheLoad, STATIC_CACHE_NAMESPACE,
};
pub use identity::{FileIdentity, StaticResourceIdentity};
pub use outcome::materialize_outcome;
pub use root::{PreloadLimits, StaticRoot};
pub use runtime::StaticRuntime;
pub use static_cache::{
    cache_hits_total as static_cache_hits_total, cache_misses_total as static_cache_misses_total,
    cache_storage_method, canonical_path_for_invalidation, canonical_path_for_snapshot,
    invalidations_total as static_cache_invalidations_total, note_cache_hit, note_cache_miss,
    note_invalidation as note_static_cache_invalidation,
    note_revalidation_failure as note_static_revalidation_failure,
    note_revalidation_success as note_static_revalidation_success,
    reset_metrics_for_tests as reset_static_cache_metrics_for_tests, revalidation_failure_total,
    revalidation_success_total, snapshot_from_identity, snapshot_matches_current,
};
#[cfg(target_os = "linux")]
pub mod epoll_bridge;
pub use kernel_hooks::install_kernel_hooks;
pub use sendfile_handle::SendfileHandleRegistry;

use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

pub const INLINE_BODY_MAX_BYTES: u64 = 1024 * 1024;

pub use bounded_read::read_bounded_from_reader;

#[derive(Debug, Error)]
pub enum StaticError {
    #[error("path traversal blocked")]
    PathTraversal,

    #[error("file not found")]
    NotFound,

    /// Materialization refused: body would exceed the request's materialization budget.
    ///
    /// Distinct from I/O / not-found. Protocol adapters map this signal (see
    /// [`exyonq_module_api::static_dispatch::MATERIALIZATION_BUDGET_EXCEEDED_HEADER`]).
    #[error("materialization budget exceeded")]
    BudgetExceeded,

    #[error("failed to read file: {0}")]
    Io(#[from] std::io::Error),
}

/// Map a request path to a file under `root`, stripping the route prefix.
/// Used in tests and one-off resolution; production hot path uses [`StaticRoot`].
pub fn resolve_path(
    root: &Path,
    route_prefix: &str,
    request_path: &str,
    index: Option<&str>,
) -> Result<PathBuf, StaticError> {
    let service = StaticRoot::new(root, route_prefix, index)?;
    service.resolve_path_sync(request_path)
}

pub async fn serve_file(path: &Path) -> Result<Response<BoxBody>, StaticError> {
    let content_type = content_type_for(path);
    let bytes = tokio::fs::read(path).await?;
    Ok(file_response(bytes::Bytes::from(bytes), content_type))
}

/// Synchronous disk read for Plan 12 cache reload (avoids stale preload bodies).
///
/// Equivalent to [`read_file_bytes_with_budget`] with `budget = None` (historical behaviour).
pub fn serve_file_sync(path: &Path) -> Result<Response<BoxBody>, StaticError> {
    let (bytes, content_type) = read_file_bytes_with_budget(path, None)?;
    Ok(file_response(bytes::Bytes::from(bytes), content_type))
}

/// Open → metadata on the same file handle → optional early reject → read bytes.
///
/// When `budget` is `Some(limit)`:
/// - if `metadata.len() > limit`, returns [`StaticError::BudgetExceeded`] without reading body bytes;
/// - otherwise reads with a bounded reader (authoritative vs TOCTOU growth).
///
/// When `budget` is `None`, preserves full-file read semantics (H1/H2 / non-budget callers).
pub fn read_file_bytes_with_budget(
    path: &Path,
    budget: Option<u64>,
) -> Result<(Vec<u8>, &'static str), StaticError> {
    let mut file = File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(StaticError::NotFound);
    }
    let content_type = content_type_for(path);
    let bytes = match budget {
        None => {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            buf
        }
        Some(limit) => {
            if meta.len() > limit {
                return Err(StaticError::BudgetExceeded);
            }
            read_bounded_from_reader(&mut file, limit)?
        }
    };
    Ok((bytes, content_type))
}

pub fn serve_head_sync(path: &Path) -> Result<Response<BoxBody>, StaticError> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(StaticError::NotFound);
    }
    let content_type = content_type_for(path);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", content_type)
        .header("content-length", meta.len())
        .body(
            http_body_util::Empty::<bytes::Bytes>::new()
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid head response"))
}

pub(crate) fn strip_route_prefix(route_prefix: &str, request_path: &str) -> Option<String> {
    if request_path == route_prefix || request_path == format!("{route_prefix}/") {
        return Some(String::new());
    }

    let prefix = route_prefix.trim_end_matches('/');
    let prefix_with_slash = format!("{prefix}/");

    if request_path.starts_with(&prefix_with_slash) {
        return Some(request_path[prefix_with_slash.len()..].to_string());
    }

    None
}

pub fn relative_as_safe_path(relative: &str) -> Result<PathBuf, StaticError> {
    let path = Path::new(relative);
    let mut safe = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(StaticError::PathTraversal);
            }
        }
    }

    Ok(safe)
}

pub fn content_type_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("txt") => "text/plain; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

pub(crate) fn file_response(body: bytes::Bytes, content_type: &'static str) -> Response<BoxBody> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", content_type)
        .header("content-length", body.len())
        .body(Full::from(body).map_err(|never| match never {}).boxed())
        .expect("valid file response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::StatusCode;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn blocks_parent_segments() {
        let err = relative_as_safe_path("../secret").unwrap_err();
        assert!(matches!(err, StaticError::PathTraversal));
    }

    #[test]
    fn serves_index_from_directory_request() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("public");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("index.html"), "<html></html>").unwrap();

        let resolved =
            resolve_path(&root, "/site", "/site", Some("index.html")).expect("index file");
        assert!(resolved.ends_with("index.html"));
    }

    #[test]
    fn blocks_path_traversal_via_dotdot() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("public");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("index.html"), "ok").unwrap();
        fs::write(dir.path().join("secret.txt"), "no").unwrap();

        let err = resolve_path(&root, "/site", "/site/../secret.txt", None).unwrap_err();
        assert!(matches!(
            err,
            StaticError::PathTraversal | StaticError::NotFound
        ));
    }

    #[tokio::test]
    async fn static_root_caches_resolved_paths() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("public");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("index.html"), "<html>hello static</html>").unwrap();
        fs::write(root.join("1k.bin"), vec![b'x'; 1024]).unwrap();

        let mut service = StaticRoot::new(&root, "/site", None).unwrap();
        service.preload_tree().unwrap();
        let index = service.serve_request("/site/").unwrap();
        assert_eq!(index.status(), StatusCode::OK);
        let first = service.serve_request("/site/1k.bin").unwrap();
        let second = service.serve_request("/site/1k.bin").unwrap();
        assert_eq!(first.status(), second.status());
        assert_eq!(service.cache_len(), 2);
    }
}
