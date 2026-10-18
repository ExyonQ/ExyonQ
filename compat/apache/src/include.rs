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
//! AP-07 / ADR-053 — offline `Include` / `IncludeOptional` expansion.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

/// Mirror `compat/nginx/src/limits.rs` (ADR-053).
pub const MAX_INCLUDE_DEPTH: usize = 16;
pub const MAX_INCLUDED_FILES: usize = 1024;
pub const MAX_TOTAL_INPUT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncludeDiag {
    /// Required Include matched nothing / missing file.
    NotFound { pattern: String },
    Cycle { path: String },
    DepthExceeded,
    Escape { path: String, detail: String },
    Budget { detail: String },
    /// IncludeOptional matched nothing — caller emits a note, not a reject.
    OptionalEmpty { pattern: String },
}

impl IncludeDiag {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "EXY-AP-INCLUDE-0002",
            Self::Cycle { .. } => "EXY-AP-INCLUDE-0003",
            Self::DepthExceeded => "EXY-AP-INCLUDE-0004",
            Self::Escape { .. } => "EXY-AP-INCLUDE-0005",
            Self::Budget { .. } => "EXY-AP-INCLUDE-0007",
            Self::OptionalEmpty { .. } => "EXY-AP-INCLUDE-0006",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotFound { pattern } => {
                format!("{}: Include `{pattern}` matched no files", self.code())
            }
            Self::Cycle { path } => format!("{}: include cycle at `{path}`", self.code()),
            Self::DepthExceeded => {
                format!(
                    "{}: include depth exceeds {MAX_INCLUDE_DEPTH}",
                    self.code()
                )
            }
            Self::Escape { path, detail } => {
                format!("{}: include `{path}` — {detail}", self.code())
            }
            Self::Budget { detail } => format!("{}: {detail}", self.code()),
            Self::OptionalEmpty { pattern } => {
                format!(
                    "{}: IncludeOptional `{pattern}` matched no files",
                    self.code()
                )
            }
        }
    }
}

pub struct IncludeCtx {
    pub entry_root: PathBuf,
    seen: HashSet<PathBuf>,
    files_loaded: usize,
    bytes_loaded: usize,
}

impl IncludeCtx {
    pub fn new(entry: &Path) -> Self {
        let parent = entry
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let entry_root = parent.canonicalize().unwrap_or(parent);
        Self {
            entry_root,
            seen: HashSet::new(),
            files_loaded: 0,
            bytes_loaded: 0,
        }
    }

    pub fn begin_file(&mut self, path: &Path, depth: usize) -> Result<(PathBuf, String), IncludeDiag> {
        if depth > MAX_INCLUDE_DEPTH {
            return Err(IncludeDiag::DepthExceeded);
        }
        if self.files_loaded >= MAX_INCLUDED_FILES {
            return Err(IncludeDiag::Budget {
                detail: format!("included file count exceeds {MAX_INCLUDED_FILES}"),
            });
        }
        let canonical = path.canonicalize().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                IncludeDiag::NotFound {
                    pattern: path.display().to_string(),
                }
            } else {
                IncludeDiag::Escape {
                    path: path.display().to_string(),
                    detail: format!("cannot resolve path: {e}"),
                }
            }
        })?;
        if !canonical.starts_with(&self.entry_root) {
            return Err(IncludeDiag::Escape {
                path: path.display().to_string(),
                detail: "path escapes allowed entry root".into(),
            });
        }
        if !self.seen.insert(canonical.clone()) {
            return Err(IncludeDiag::Cycle {
                path: canonical.display().to_string(),
            });
        }
        let content = std::fs::read_to_string(&canonical).map_err(|e| IncludeDiag::NotFound {
            pattern: format!("{} ({e})", canonical.display()),
        })?;
        self.bytes_loaded = self.bytes_loaded.saturating_add(content.len());
        if self.bytes_loaded > MAX_TOTAL_INPUT_BYTES {
            return Err(IncludeDiag::Budget {
                detail: format!("total input exceeds {MAX_TOTAL_INPUT_BYTES} bytes"),
            });
        }
        self.files_loaded += 1;
        Ok((canonical, content))
    }

    /// Resolve Include pattern relative to `including_file`'s parent.
    pub fn resolve_targets(
        &self,
        including_file: &Path,
        pattern: &str,
        optional: bool,
    ) -> Result<Vec<PathBuf>, IncludeDiag> {
        let base = including_file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.entry_root.clone());
        let paths = expand_glob(&base, pattern, &self.entry_root)?;
        if paths.is_empty() {
            if optional {
                return Err(IncludeDiag::OptionalEmpty {
                    pattern: pattern.to_string(),
                });
            }
            return Err(IncludeDiag::NotFound {
                pattern: pattern.to_string(),
            });
        }
        Ok(paths)
    }
}

fn expand_glob(
    base: &Path,
    pattern: &str,
    entry_root: &Path,
) -> Result<Vec<PathBuf>, IncludeDiag> {
    let raw = Path::new(pattern);
    if raw.is_absolute() {
        let canonical = raw.canonicalize().map_err(|e| IncludeDiag::Escape {
            path: pattern.to_string(),
            detail: format!("cannot resolve absolute include: {e}"),
        })?;
        if !canonical.starts_with(entry_root) {
            return Err(IncludeDiag::Escape {
                path: pattern.to_string(),
                detail: "absolute include escapes entry root".into(),
            });
        }
        return Ok(vec![canonical]);
    }

    if pattern.contains('*') {
        let pattern_path = PathBuf::from(pattern);
        let parent_rel = pattern_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let file_glob = pattern_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| pattern.to_string());
        if file_glob.matches('*').count() != 1 {
            return Err(IncludeDiag::Escape {
                path: pattern.to_string(),
                detail: "only a single '*' in the last path component is supported".into(),
            });
        }
        let dir = safe_join(base, &parent_rel)?;
        let dir = dir.canonicalize().unwrap_or(dir);
        if !dir.starts_with(entry_root) {
            return Err(IncludeDiag::Escape {
                path: pattern.to_string(),
                detail: "glob directory escapes entry root".into(),
            });
        }
        let prefix = file_glob.split('*').next().unwrap_or("");
        let suffix = file_glob.rsplit('*').next().unwrap_or("");
        let mut matches = Vec::new();
        let entries = std::fs::read_dir(&dir).map_err(|e| IncludeDiag::Escape {
            path: pattern.to_string(),
            detail: format!("cannot read include directory: {e}"),
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| IncludeDiag::Escape {
                path: pattern.to_string(),
                detail: format!("cannot read directory entry: {e}"),
            })?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(prefix) && name.ends_with(suffix) {
                matches.push(entry.path());
            }
        }
        matches.sort();
        return Ok(matches);
    }

    let joined = safe_join(base, raw)?;
    if !joined.exists() {
        // Empty → caller maps to NotFound (Include) or OptionalEmpty.
        return Ok(vec![]);
    }
    Ok(vec![joined.canonicalize().unwrap_or(joined)])
}

fn safe_join(base: &Path, rel: &Path) -> Result<PathBuf, IncludeDiag> {
    let mut cur = base.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(IncludeDiag::Escape {
                    path: rel.display().to_string(),
                    detail: "path traversal (`..`) in include".into(),
                });
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(IncludeDiag::Escape {
                    path: rel.display().to_string(),
                    detail: "unexpected absolute component in relative include".into(),
                });
            }
            Component::Normal(part) => {
                cur.push(part);
            }
        }
    }
    // Prefer canonical when the path exists; otherwise keep the joined path so
    // a later read surfaces NotFound (required Include) instead of Escape.
    Ok(cur.canonicalize().unwrap_or(cur))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_empty_is_distinct_code() {
        let d = IncludeDiag::OptionalEmpty {
            pattern: "missing.conf".into(),
        };
        assert_eq!(d.code(), "EXY-AP-INCLUDE-0006");
    }

    #[test]
    fn rejects_parent_dir_traversal() {
        let err = safe_join(Path::new("/tmp"), Path::new("../etc/passwd")).unwrap_err();
        assert_eq!(err.code(), "EXY-AP-INCLUDE-0005");
    }
}
