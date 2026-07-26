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
//! Generic path helpers for static route compilation (KD2.5 — no module types in core).

use std::path::{Component, Path, PathBuf};

/// Path resolution error shared by core routing seams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaticPathError {
    PathTraversal,
    NotFound,
}

/// Normalize a relative URI segment without `..` or absolute roots.
pub fn relative_as_safe_path(relative: &str) -> Result<PathBuf, StaticPathError> {
    let path = Path::new(relative);
    let mut safe = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(StaticPathError::PathTraversal);
            }
        }
    }

    Ok(safe)
}

/// Strip a compiled route prefix from a request path (returns relative segment).
pub fn strip_route_prefix(route_prefix: &str, request_path: &str) -> Option<String> {
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

/// Map request URI to filesystem path under a compiled static root (sync probe only).
pub fn resolve_path_under_root(
    filesystem_root: &Path,
    route_prefix: &str,
    request_path: &str,
    index: Option<&str>,
) -> Result<PathBuf, StaticPathError> {
    let relative =
        strip_route_prefix(route_prefix, request_path).ok_or(StaticPathError::NotFound)?;
    let mut path = filesystem_root.to_path_buf();
    if relative.is_empty() {
        if let Some(index_name) = index {
            path.push(index_name);
        } else {
            return Err(StaticPathError::NotFound);
        }
    } else {
        let safe = relative_as_safe_path(&relative)?;
        path.push(safe);
    }
    if path.starts_with(filesystem_root) {
        Ok(path)
    } else {
        Err(StaticPathError::PathTraversal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_parent_segments() {
        assert_eq!(
            relative_as_safe_path("../secret"),
            Err(StaticPathError::PathTraversal)
        );
    }

    #[test]
    fn resolves_index_request() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("public");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("index.html"), b"ok").expect("write");
        let resolved =
            resolve_path_under_root(&root, "/site", "/site", Some("index.html")).expect("path");
        assert!(resolved.ends_with("index.html"));
    }
}
