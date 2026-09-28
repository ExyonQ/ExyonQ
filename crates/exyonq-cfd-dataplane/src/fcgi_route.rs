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
//! ADR-043: generic directory-index + front-controller resolution for CFD FastCGI.
//!
//! Selects the executable script URI before `execute_get`. Never takes SCRIPT_FILENAME
//! from the client. Preserves the original request URI at the caller.

use std::fs;
use std::path::{Component, Path, PathBuf};

use exyonq_cfd_gen::{join_directory_index_uri, CompiledFcgiPool, CompiledFrontControllerPolicy};

/// Outcome of CFD FastCGI script resolution (ADR-043).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FcgiRouteResolve {
    /// Execute this script URI (absolute path under docroot, starts with `/`).
    Script { script_uri: String },
    /// Genuine application miss (no FC / DI hit).
    NotFound,
    /// Configured FC/DI target missing or not an executable `.php` file.
    Misconfigured,
    /// Path escaped docroot / invalid components / NUL.
    Forbidden,
}

/// Resolve which PHP script to execute for a FastCGI-matched request path.
///
/// `request_path` is path-only (no query). When the pool has no DI and no FC policy,
/// returns `Script { script_uri: request_path }` so legacy `script_filename_under_root`
/// behavior (including `/` → `/index.php`) is preserved.
pub fn resolve_fcgi_script(pool: &CompiledFcgiPool, request_path: &str) -> FcgiRouteResolve {
    if request_path.contains('\0') || pool.document_root.contains('\0') {
        return FcgiRouteResolve::Forbidden;
    }
    let path = normalize_request_path(request_path);
    let has_policy = !pool.directory_index.is_empty() || pool.front_controller.is_some();
    if !has_policy {
        return FcgiRouteResolve::Script {
            script_uri: path.to_string(),
        };
    }

    let Ok(root_canon) = canonicalize_root(&pool.document_root) else {
        return FcgiRouteResolve::Misconfigured;
    };

    let joined = match join_under_root(&root_canon, &path) {
        Ok(p) => p,
        Err(()) => return FcgiRouteResolve::Forbidden,
    };

    match classify_path(&joined) {
        PathClass::Missing => {
            // Front-controller fallback for non-existent paths.
            try_front_controller(pool, &root_canon, &path)
        }
        PathClass::File => {
            if !path_is_under_root(&root_canon, &joined) {
                return FcgiRouteResolve::Forbidden;
            }
            if is_php_uri(&path) {
                FcgiRouteResolve::Script {
                    script_uri: path.to_string(),
                }
            } else {
                // Existing non-PHP on FastCGI route: do not execute / disclose.
                FcgiRouteResolve::NotFound
            }
        }
        PathClass::Directory => {
            let dir_uri = if path.ends_with('/') {
                path.to_string()
            } else {
                format!("{path}/")
            };
            match try_directory_index(pool, &root_canon, &dir_uri) {
                FcgiRouteResolve::NotFound => {
                    // Directory exists → FC require_not_directory blocks FC.
                    FcgiRouteResolve::NotFound
                }
                other => other,
            }
        }
        PathClass::Other => FcgiRouteResolve::Forbidden,
    }
}

#[derive(Debug)]
enum PathClass {
    Missing,
    File,
    Directory,
    Other,
}

fn normalize_request_path(request_path: &str) -> String {
    if request_path.is_empty() || request_path == "/" {
        return "/".into();
    }
    if request_path.starts_with('/') {
        request_path.to_string()
    } else {
        format!("/{request_path}")
    }
}

fn is_php_uri(uri: &str) -> bool {
    uri.ends_with(".php")
}

fn canonicalize_root(document_root: &str) -> Result<PathBuf, ()> {
    let root = Path::new(document_root);
    if !root.is_absolute() {
        return Err(());
    }
    fs::canonicalize(root).map_err(|_| ())
}

fn join_under_root(root_canon: &Path, request_path: &str) -> Result<PathBuf, ()> {
    let rel = request_path.trim_start_matches('/');
    let mut joined = root_canon.to_path_buf();
    if !rel.is_empty() {
        for c in Path::new(rel).components() {
            match c {
                Component::Normal(s) => joined.push(s),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(());
                }
            }
        }
    }
    Ok(joined)
}

fn classify_path(path: &Path) -> PathClass {
    match fs::symlink_metadata(path) {
        Err(_) => PathClass::Missing,
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                // Resolve symlink targets only when they stay under root via canonicalize.
                match fs::canonicalize(path) {
                    Ok(canon) => {
                        if meta.is_dir() || canon.is_dir() {
                            PathClass::Directory
                        } else if canon.is_file() {
                            PathClass::File
                        } else {
                            PathClass::Other
                        }
                    }
                    Err(_) => PathClass::Other,
                }
            } else if meta.is_dir() {
                PathClass::Directory
            } else if meta.is_file() {
                PathClass::File
            } else {
                PathClass::Other
            }
        }
    }
}

fn path_is_under_root(root_canon: &Path, candidate: &Path) -> bool {
    match fs::canonicalize(candidate) {
        Ok(canon) => canon.starts_with(root_canon),
        Err(_) => false,
    }
}

fn try_directory_index(
    pool: &CompiledFcgiPool,
    root_canon: &Path,
    dir_uri: &str,
) -> FcgiRouteResolve {
    for name in &pool.directory_index {
        let Some(child_uri) = join_directory_index_uri(dir_uri, name) else {
            return FcgiRouteResolve::Forbidden;
        };
        let Ok(child_path) = join_under_root(root_canon, &child_uri) else {
            return FcgiRouteResolve::Forbidden;
        };
        match classify_path(&child_path) {
            PathClass::File => {
                if !path_is_under_root(root_canon, &child_path) {
                    return FcgiRouteResolve::Forbidden;
                }
                if is_php_uri(&child_uri) {
                    return FcgiRouteResolve::Script {
                        script_uri: child_uri,
                    };
                }
                // Non-PHP index: not FastCGI-served on this path (no source disclosure).
                return FcgiRouteResolve::NotFound;
            }
            PathClass::Missing => continue,
            PathClass::Directory | PathClass::Other => continue,
        }
    }
    FcgiRouteResolve::NotFound
}

fn try_front_controller(
    pool: &CompiledFcgiPool,
    root_canon: &Path,
    request_path: &str,
) -> FcgiRouteResolve {
    let Some(fc) = pool.front_controller.as_ref() else {
        return FcgiRouteResolve::NotFound;
    };
    // require_not_file / require_not_directory already implied by Missing class.
    let _ = request_path;
    let _ = fc.require_not_file;
    let _ = fc.require_not_directory;
    resolve_fc_target(root_canon, fc)
}

fn resolve_fc_target(root_canon: &Path, fc: &CompiledFrontControllerPolicy) -> FcgiRouteResolve {
    let Ok(target_path) = join_under_root(root_canon, &fc.target_uri) else {
        return FcgiRouteResolve::Forbidden;
    };
    match classify_path(&target_path) {
        PathClass::File => {
            if !path_is_under_root(root_canon, &target_path) {
                return FcgiRouteResolve::Forbidden;
            }
            if !is_php_uri(&fc.target_uri) {
                return FcgiRouteResolve::Misconfigured;
            }
            FcgiRouteResolve::Script {
                script_uri: fc.target_uri.clone(),
            }
        }
        PathClass::Missing | PathClass::Directory | PathClass::Other => {
            FcgiRouteResolve::Misconfigured
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use exyonq_cfd_gen::{CompiledFcgiPool, CompiledFrontControllerPolicy, FcgiTransport};
    use tempfile::tempdir;

    fn pool(root: &Path, di: &[&str], fc: Option<&str>) -> CompiledFcgiPool {
        CompiledFcgiPool {
            id: 1,
            transport: FcgiTransport::Tcp(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000)),
            document_root: root.to_string_lossy().into_owned(),
            script_suffix: String::new(),
            max_connections: 1,
            idle_timeout_ms: 60_000,
            connect_timeout_ms: 2_000,
            read_timeout_ms: 5_000,
            write_timeout_ms: 5_000,
            total_timeout_ms: 10_000,
            directory_index: di.iter().map(|s| (*s).to_string()).collect(),
            front_controller: fc.map(|t| CompiledFrontControllerPolicy {
                target_uri: t.to_string(),
                require_not_file: true,
                require_not_directory: true,
                preserve_query: true,
            }),
        }
    }

    #[test]
    fn legacy_without_policy_passes_path() {
        let dir = tempdir().unwrap();
        let p = pool(dir.path(), &[], None);
        assert_eq!(
            resolve_fcgi_script(&p, "/x.php"),
            FcgiRouteResolve::Script {
                script_uri: "/x.php".into()
            }
        );
    }

    #[test]
    fn directory_index_selects_php() {
        let dir = tempdir().unwrap();
        let admin = dir.path().join("admin");
        fs::create_dir(&admin).unwrap();
        fs::write(admin.join("index.php"), b"<?php echo 1;").unwrap();
        let p = pool(dir.path(), &["index.php", "index.html"], None);
        assert_eq!(
            resolve_fcgi_script(&p, "/admin/"),
            FcgiRouteResolve::Script {
                script_uri: "/admin/index.php".into()
            }
        );
    }

    #[test]
    fn front_controller_on_missing_path() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.php"), b"<?php").unwrap();
        let p = pool(dir.path(), &["index.php"], Some("/index.php"));
        assert_eq!(
            resolve_fcgi_script(&p, "/pretty/path/"),
            FcgiRouteResolve::Script {
                script_uri: "/index.php".into()
            }
        );
    }

    #[test]
    fn existing_php_bypasses_front_controller() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.php"), b"<?php").unwrap();
        fs::write(dir.path().join("login.php"), b"<?php").unwrap();
        let p = pool(dir.path(), &["index.php"], Some("/index.php"));
        assert_eq!(
            resolve_fcgi_script(&p, "/login.php"),
            FcgiRouteResolve::Script {
                script_uri: "/login.php".into()
            }
        );
    }

    #[test]
    fn existing_static_not_fcgi_executed() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.php"), b"<?php").unwrap();
        fs::write(dir.path().join("style.css"), b"body{}").unwrap();
        let p = pool(dir.path(), &["index.php"], Some("/index.php"));
        assert_eq!(
            resolve_fcgi_script(&p, "/style.css"),
            FcgiRouteResolve::NotFound
        );
    }

    #[test]
    fn missing_without_fc_is_not_found() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.php"), b"<?php").unwrap();
        let p = pool(dir.path(), &["index.php"], None);
        assert_eq!(
            resolve_fcgi_script(&p, "/missing/"),
            FcgiRouteResolve::NotFound
        );
    }

    #[test]
    fn traversal_forbidden() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.php"), b"<?php").unwrap();
        let p = pool(dir.path(), &["index.php"], Some("/index.php"));
        assert_eq!(
            resolve_fcgi_script(&p, "/../index.php"),
            FcgiRouteResolve::Forbidden
        );
    }

    #[test]
    fn explicit_php_symlink_outside_root_forbidden() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.php"), b"<?php echo SECRET;").unwrap();
        fs::write(dir.path().join("index.php"), b"<?php").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                outside.path().join("secret.php"),
                dir.path().join("link.php"),
            )
            .unwrap();
            let p = pool(dir.path(), &["index.php"], Some("/index.php"));
            assert_eq!(
                resolve_fcgi_script(&p, "/link.php"),
                FcgiRouteResolve::Forbidden
            );
        }
    }

    #[test]
    fn directory_index_non_php_not_disclosed_via_fcgi() {
        let dir = tempdir().unwrap();
        let admin = dir.path().join("admin");
        fs::create_dir(&admin).unwrap();
        fs::write(admin.join("index.html"), b"<html>").unwrap();
        let p = pool(dir.path(), &["index.html"], None);
        assert_eq!(
            resolve_fcgi_script(&p, "/admin/"),
            FcgiRouteResolve::NotFound
        );
    }
}
