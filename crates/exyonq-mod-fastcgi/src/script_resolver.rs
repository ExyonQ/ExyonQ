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
//! KD4.7 — FastCGI script target resolution (module-owned policy).

use exyonq_module_api::fcgi_script_resolver::{
    FastcgiScriptResolutionOutcome, FastcgiScriptResolutionRequest, FastcgiScriptResolutionService,
};
use exyonq_module_api::static_paths::{relative_as_safe_path, StaticPathError};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolveError {
    EmptyUriPath,
    InvalidDocumentRoot,
    InvalidPath,
    NotFound,
    ProbeError,
}

impl From<StaticPathError> for ResolveError {
    fn from(err: StaticPathError) -> Self {
        match err {
            StaticPathError::PathTraversal => ResolveError::InvalidPath,
            StaticPathError::NotFound => ResolveError::NotFound,
        }
    }
}

#[derive(Clone)]
pub struct FastcgiScriptResolver {
    env_document_root: Option<PathBuf>,
}

impl FastcgiScriptResolver {
    pub fn new() -> Self {
        let env_document_root = std::env::var("EXYONQ_FCGI_DOCUMENT_ROOT")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        if env_document_root.is_some() {
            tracing::warn!(
                "EXYONQ_FCGI_DOCUMENT_ROOT is deprecated; set fcgi_pool.document_root in config"
            );
        }
        Self { env_document_root }
    }

    pub fn arc() -> Arc<dyn FastcgiScriptResolutionService> {
        Arc::new(Self::new())
    }

    fn pick_document_root(&self, pool_document_root: Option<&PathBuf>) -> Option<PathBuf> {
        pool_document_root
            .filter(|p| !p.as_os_str().is_empty())
            .cloned()
            .or_else(|| self.env_document_root.clone())
    }
}

impl Default for FastcgiScriptResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl FastcgiScriptResolutionService for FastcgiScriptResolver {
    fn resolve(&self, request: &FastcgiScriptResolutionRequest) -> FastcgiScriptResolutionOutcome {
        // EXY-03: route-level `alias` script root takes precedence over the
        // pool document root (uri_path then carries the route remainder).
        let root_source = request
            .script_root
            .as_ref()
            .or(request.pool_document_root.as_ref());
        let document_root = match self.pick_document_root(root_source) {
            Some(root) => root,
            // No document root at all is a pool/route misconfiguration — an
            // upstream-side fault (502 via ProbeError), not a client 403.
            None => {
                return FastcgiScriptResolutionOutcome::ProbeError;
            }
        };

        // EXY-03: `location =` + file alias — an empty remainder resolves the
        // script root itself when it names a regular file (verbatim target).
        if request.script_root.is_some() && request.uri_path.trim_start_matches('/').is_empty() {
            return match document_root.canonicalize().ok().filter(|p| p.is_file()) {
                Some(script) => FastcgiScriptResolutionOutcome::Resolved {
                    script_filename: script.display().to_string(),
                    script_name: request
                        .script_name_uri
                        .clone()
                        .unwrap_or_else(|| request.uri_path.clone()),
                    path_info: None,
                    document_root: document_root.display().to_string(),
                },
                None => FastcgiScriptResolutionOutcome::NotFound,
            };
        }

        match resolve_under_root(&document_root, &request.uri_path) {
            Ok(resolved) => FastcgiScriptResolutionOutcome::Resolved {
                script_filename: resolved.script_filename,
                script_name: request
                    .script_name_uri
                    .clone()
                    .unwrap_or(resolved.script_name),
                path_info: resolved.path_info,
                document_root: resolved.document_root,
            },
            Err(ResolveError::NotFound) => {
                match front_controller_fallback(
                    &document_root,
                    &request.uri_path,
                    request.front_controller.as_deref(),
                ) {
                    Some(resolved) => FastcgiScriptResolutionOutcome::Resolved {
                        script_filename: resolved.script_filename,
                        script_name: request
                            .script_name_uri
                            .clone()
                            .unwrap_or(resolved.script_name),
                        path_info: resolved.path_info,
                        document_root: resolved.document_root,
                    },
                    None => FastcgiScriptResolutionOutcome::NotFound,
                }
            }
            Err(ResolveError::InvalidPath | ResolveError::EmptyUriPath) => {
                FastcgiScriptResolutionOutcome::InvalidPath
            }
            Err(ResolveError::ProbeError | ResolveError::InvalidDocumentRoot) => {
                FastcgiScriptResolutionOutcome::ProbeError
            }
        }
    }
}

#[derive(Debug)]
struct Resolved {
    script_filename: String,
    script_name: String,
    path_info: Option<String>,
    document_root: String,
}

fn resolve_under_root(document_root: &Path, uri_path: &str) -> Result<Resolved, ResolveError> {
    if uri_path.is_empty() || !uri_path.starts_with('/') {
        return Err(ResolveError::EmptyUriPath);
    }
    if uri_path.contains('\0') {
        return Err(ResolveError::InvalidPath);
    }

    let canonical_root = document_root
        .canonicalize()
        .map_err(|_| ResolveError::InvalidDocumentRoot)?;

    // Remove trailing slashes to avoid treating "/index.php/" as a directory target.
    let trimmed = uri_path.trim_end_matches('/');
    let relative = trimmed.trim_start_matches('/');
    if relative.is_empty() {
        return Err(ResolveError::NotFound);
    }

    // PATH_INFO semantics: find the deepest prefix that exists as a regular file.
    // The remainder (if any) becomes PATH_INFO. Percent-decoding is not performed.
    let parts: Vec<&str> = relative.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Err(ResolveError::NotFound);
    }

    let mut resolved: Option<(PathBuf, String, Option<String>)> = None;
    for cut in (1..=parts.len()).rev() {
        let script_rel = parts[..cut].join("/");
        let safe_rel = relative_as_safe_path(&script_rel)?;
        let candidate = canonical_root.join(safe_rel);
        let meta = match std::fs::metadata(&candidate) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            continue;
        }
        if !meta.is_file() {
            continue;
        }

        let path_info = if cut == parts.len() {
            None
        } else {
            Some(format!("/{}", parts[cut..].join("/")))
        };
        let script_uri = format!("/{}", script_rel);
        resolved = Some((candidate, script_uri, path_info));
        break;
    }

    let Some((candidate, script_uri, path_info)) = resolved else {
        return Err(ResolveError::NotFound);
    };

    let canonical = candidate
        .canonicalize()
        .map_err(|_| ResolveError::ProbeError)?;
    if !canonical.starts_with(&canonical_root) {
        return Err(ResolveError::InvalidPath);
    }
    if !canonical.is_file() {
        return Err(ResolveError::NotFound);
    }

    let script_name = normalize_script_name(&script_uri);
    let script_filename = canonical.to_string_lossy().into_owned();
    Ok(Resolved {
        script_filename,
        script_name,
        path_info,
        document_root: canonical_root.to_string_lossy().into_owned(),
    })
}

/// EXY-10: `try_files $uri $uri/ /index.php?$args` fallback, resolved under the
/// same root the normal lookup used (pool `document_root` or route
/// `fastcgi_script_root`). Two steps, both bounded inside the root:
///
/// 1. *Directory index* — the URI names an existing directory (`/`, `/wp-admin/`):
///    `<dir>/<name>` wins when it is a regular file; `PATH_INFO` stays empty.
/// 2. *Front controller* — anything else (`/2026/10/hola/`, a missing `.php`):
///    `<root>/<name>` with `PATH_INFO` = the original `uri_path`.
///
/// Returns `None` when `front_controller` is unset or the named script does not
/// exist — the caller still sees `NotFound`.
fn front_controller_fallback(
    document_root: &Path,
    uri_path: &str,
    front_controller: Option<&str>,
) -> Option<Resolved> {
    let fc_name = front_controller?.trim_matches('/');
    if fc_name.is_empty() {
        return None;
    }
    let fc_rel = relative_as_safe_path(fc_name).ok()?;
    let canonical_root = document_root.canonicalize().ok()?;
    let resolved_root = || canonical_root.to_string_lossy().into_owned();

    // Step 1: directory index — URI is `/` or resolves to an existing dir.
    let rel = uri_path.trim_matches('/');
    let dir = if rel.is_empty() {
        Some((canonical_root.clone(), String::new()))
    } else {
        relative_as_safe_path(rel)
            .ok()
            .map(|safe| (canonical_root.join(&safe), rel.to_string()))
            .filter(|(p, _)| p.is_dir())
    };
    if let Some((dir, dir_uri)) = dir {
        let idx = dir.join(&fc_rel);
        let script_name = if dir_uri.is_empty() {
            format!("/{fc_name}")
        } else {
            format!("/{dir_uri}/{fc_name}")
        };
        if let Some(resolved) =
            resolve_script_file(&canonical_root, &idx, resolved_root(), &script_name)
        {
            return Some(resolved);
        }
    }

    // Step 2: front controller at the root — `PATH_INFO` carries the full URI.
    let mut resolved = resolve_script_file(
        &canonical_root,
        &canonical_root.join(&fc_rel),
        resolved_root(),
        &format!("/{fc_name}"),
    )?;
    resolved.path_info = Some(uri_path.to_string());
    Some(resolved)
}

/// Canonicalize + bound a candidate script under `canonical_root`.
fn resolve_script_file(
    canonical_root: &Path,
    candidate: &Path,
    document_root: String,
    script_name: &str,
) -> Option<Resolved> {
    let canonical = candidate.canonicalize().ok()?;
    if !canonical.starts_with(canonical_root) || !canonical.is_file() {
        return None;
    }
    Some(Resolved {
        script_filename: canonical.to_string_lossy().into_owned(),
        script_name: normalize_script_name(script_name),
        path_info: None,
        document_root,
    })
}

fn normalize_script_name(uri_path: &str) -> String {
    let path = Path::new(uri_path);
    let mut out = String::from("/");
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if out.len() > 1 {
                    out.push('/');
                }
                out.push_str(&part.to_string_lossy());
            }
            Component::RootDir => {}
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => {}
        }
    }
    if out == "/" && uri_path != "/" {
        uri_path.to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_module_api::fcgi_script_resolver::{
        FastcgiScriptResolutionPurpose, FastcgiScriptResolutionRequest,
    };
    use std::fs;
    use tempfile::tempdir;

    fn req(path: &str, root: &Path) -> FastcgiScriptResolutionRequest {
        FastcgiScriptResolutionRequest {
            uri_path: path.to_string(),
            pool_document_root: Some(root.to_path_buf()),
            script_root: None,
            script_name_uri: None,
            front_controller: None,
            generation: 0,
            purpose: FastcgiScriptResolutionPurpose::Dispatch,
        }
    }

    fn req_fc(path: &str, root: &Path, fc: &str) -> FastcgiScriptResolutionRequest {
        FastcgiScriptResolutionRequest {
            front_controller: Some(fc.to_string()),
            ..req(path, root)
        }
    }

    #[test]
    fn resolves_existing_file_under_root() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("index.php"), b"<?php").expect("write");
        let out = resolve_under_root(root, "/index.php").expect("resolve");
        assert_eq!(out.script_name, "/index.php");
        assert!(out.script_filename.ends_with("index.php"));
        assert!(out.path_info.is_none());
    }

    #[test]
    fn blocks_traversal() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("secret.php"), b"x").expect("write");
        let err = resolve_under_root(root, "/../secret.php").unwrap_err();
        assert_eq!(err, ResolveError::InvalidPath);
    }

    #[test]
    fn rejects_missing_file() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let err = resolve_under_root(root, "/missing.php").unwrap_err();
        assert_eq!(err, ResolveError::NotFound);
    }

    #[test]
    fn resolves_path_info_by_suffix_stripping() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("index.php"), b"<?php").expect("write");
        let out = resolve_under_root(root, "/index.php/posts/hello").expect("resolve");
        assert_eq!(out.script_name, "/index.php");
        assert_eq!(out.path_info.as_deref(), Some("/posts/hello"));
    }

    #[test]
    fn front_controller_serves_root_index() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("index.php"), b"<?php").expect("write");
        let svc = FastcgiScriptResolver::new();
        let out = resolve_under_root(root, "/");
        assert!(matches!(out, Err(ResolveError::NotFound)));
        match svc.resolve(&req_fc("/", root, "index.php")) {
            FastcgiScriptResolutionOutcome::Resolved {
                script_name,
                path_info,
                script_filename,
                ..
            } => {
                assert_eq!(script_name, "/index.php");
                assert!(path_info.is_none(), "directory index carries no PATH_INFO");
                assert!(script_filename.ends_with("index.php"));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn front_controller_directory_index_wins_over_fallback() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("index.php"), b"<?php root").expect("write");
        fs::create_dir(root.join("wp-admin")).expect("mkdir");
        fs::write(root.join("wp-admin").join("index.php"), b"<?php admin").expect("write");
        let svc = FastcgiScriptResolver::new();
        match svc.resolve(&req_fc("/wp-admin/", root, "index.php")) {
            FastcgiScriptResolutionOutcome::Resolved {
                script_name,
                path_info,
                script_filename,
                ..
            } => {
                assert_eq!(script_name, "/wp-admin/index.php");
                assert!(path_info.is_none());
                assert!(script_filename.ends_with("wp-admin/index.php"));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn front_controller_permalink_sets_path_info() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("index.php"), b"<?php").expect("write");
        let svc = FastcgiScriptResolver::new();
        match svc.resolve(&req_fc("/2026/10/hola-mundo/", root, "index.php")) {
            FastcgiScriptResolutionOutcome::Resolved {
                script_name,
                path_info,
                ..
            } => {
                assert_eq!(script_name, "/index.php");
                assert_eq!(path_info.as_deref(), Some("/2026/10/hola-mundo/"));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn front_controller_missing_script_stays_not_found() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let svc = FastcgiScriptResolver::new();
        let outcome = svc.resolve(&req_fc("/", root, "index.php"));
        assert_eq!(outcome, FastcgiScriptResolutionOutcome::NotFound);
    }

    #[test]
    fn no_front_controller_keeps_not_found() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("index.php"), b"<?php").expect("write");
        let svc = FastcgiScriptResolver::new();
        let outcome = svc.resolve(&req("/", root));
        assert_eq!(outcome, FastcgiScriptResolutionOutcome::NotFound);
    }

    #[test]
    fn front_controller_traversal_stays_invalid() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("index.php"), b"<?php").expect("write");
        fs::write(root.join("secret.php"), b"x").expect("write");
        let svc = FastcgiScriptResolver::new();
        let outcome = svc.resolve(&req_fc("/../secret.php", root, "index.php"));
        assert_eq!(outcome, FastcgiScriptResolutionOutcome::InvalidPath);
    }

    #[test]
    fn front_controller_missing_php_also_falls_back() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("index.php"), b"<?php").expect("write");
        let svc = FastcgiScriptResolver::new();
        match svc.resolve(&req_fc("/missing.php", root, "index.php")) {
            FastcgiScriptResolutionOutcome::Resolved {
                script_name,
                path_info,
                ..
            } => {
                assert_eq!(script_name, "/index.php");
                assert_eq!(path_info.as_deref(), Some("/missing.php"));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn service_maps_outcomes() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("index.php"), b"<?php").expect("write");
        let svc = FastcgiScriptResolver::new();
        let outcome = svc.resolve(&req("/index.php/x", root));
        match outcome {
            FastcgiScriptResolutionOutcome::Resolved { path_info, .. } => {
                assert_eq!(path_info.as_deref(), Some("/x"));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }
}
