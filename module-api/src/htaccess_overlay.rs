//! Plan 05B — immutable `.htaccess` overlay contract (read-only at runtime).

use std::sync::Arc;

/// Maximum compiled `DirectoryIndex` candidates per directory (Plan 11).
pub const MAX_DIRECTORY_INDEX_CANDIDATES: usize = 32;

/// Maximum length of a single directory-index candidate filename.
pub const MAX_DIRECTORY_INDEX_CANDIDATE_LEN: usize = 255;

/// Maximum compiled front-controller target URI length (Plan 11 tranche C).
pub const MAX_FRONT_CONTROLLER_TARGET_LEN: usize = 2048;

/// Compiled `RewriteCond !-f/!-d` + `RewriteRule` front-controller subset (no regex runtime).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledFrontController {
    pub target_uri: Arc<str>,
    pub require_not_file: bool,
    pub require_not_directory: bool,
    pub preserve_query: bool,
}

/// Validate one directory-index candidate filename (not a URI path).
pub fn validate_directory_index_candidate(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_DIRECTORY_INDEX_CANDIDATE_LEN {
        return false;
    }
    if name.starts_with('/')
        || name.contains("..")
        || name.contains('\0')
        || name.contains('\\')
        || name.contains('/')
    {
        return false;
    }
    true
}

/// Join a directory URI ending in `/` with a safe candidate → child URI.
pub fn join_directory_index_uri(dir_uri: &str, candidate: &str) -> Option<String> {
    if !uri_qualifies_for_directory_index(dir_uri) || !validate_directory_index_candidate(candidate)
    {
        return None;
    }
    if dir_uri == "/" {
        Some(format!("/{candidate}"))
    } else {
        Some(format!("{dir_uri}{candidate}"))
    }
}

fn uri_qualifies_for_directory_index(path: &str) -> bool {
    path.ends_with('/')
}

/// Canonical directory key relative to vhost document root (always `/` or `/subdir`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NormalizedDirectory(pub String);

/// Literal redirect compiled offline (`Redirect 301 /old /new`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledRedirectRule {
    pub from_path: String,
    pub status: u16,
    pub location: String,
}

/// Safe external rewrite redirect (`RewriteRule ^old$ /new [R=301,L]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledRewriteRedirect {
    pub from_path: String,
    pub status: u16,
    pub location: String,
}

/// Precompiled overlay for one directory after inheritance merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayEntry {
    pub directory: NormalizedDirectory,
    pub directory_index: Option<Arc<[String]>>,
    pub redirect_rules: Arc<[CompiledRedirectRule]>,
    pub rewrite_redirects: Arc<[CompiledRewriteRedirect]>,
    pub indexes_disabled: bool,
    pub front_controller: Option<CompiledFrontController>,
}

/// Immutable per-vhost overlay published atomically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VhostOverlay {
    pub generation: u64,
    pub site_id: String,
    pub document_root: String,
    pub entries: Arc<[OverlayEntry]>,
}

/// Outcome of read-only overlay lookup (no I/O, no parsing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayLookupResult {
    Miss,
    Redirect {
        status: u16,
        location: String,
    },
    Continue {
        directory_index: Option<Arc<[String]>>,
        indexes_disabled: bool,
        front_controller: Option<CompiledFrontController>,
    },
}

/// Publish envelope validated against active plan generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePatchVhostOverlay {
    pub site_id: String,
    pub plan_generation: u64,
    pub overlay: VhostOverlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayPublishError {
    ObsoletePlanGeneration,
    InvalidOverlay,
    SiteIdMismatch,
}

pub fn normalize_uri_path(path: &str) -> String {
    if path.is_empty() {
        return "/".into();
    }
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

/// Longest matching directory entry, then exact redirect rules (v0).
pub fn lookup_overlay(uri_path: &str, overlay: &VhostOverlay) -> OverlayLookupResult {
    let path = normalize_uri_path(uri_path);
    let entry = overlay
        .entries
        .iter()
        .filter(|e| path.starts_with(e.directory.0.as_str()))
        .max_by_key(|e| e.directory.0.len());

    let Some(entry) = entry else {
        return OverlayLookupResult::Miss;
    };

    for rule in entry.redirect_rules.iter() {
        if rule.from_path == path {
            return OverlayLookupResult::Redirect {
                status: rule.status,
                location: rule.location.clone(),
            };
        }
    }
    for rule in entry.rewrite_redirects.iter() {
        if rule.from_path == path {
            return OverlayLookupResult::Redirect {
                status: rule.status,
                location: rule.location.clone(),
            };
        }
    }

    OverlayLookupResult::Continue {
        directory_index: entry.directory_index.clone(),
        indexes_disabled: entry.indexes_disabled,
        front_controller: entry.front_controller.clone(),
    }
}

/// Validate a compiled front-controller target URI (local, literal, bounded).
pub fn validate_front_controller_target(target: &str) -> bool {
    if target.is_empty()
        || target.len() > MAX_FRONT_CONTROLLER_TARGET_LEN
        || !target.starts_with('/')
        || target == "/"
        || target.contains("..")
        || target.contains('\0')
        || target.contains('$')
        || target.contains('%')
        || target.starts_with("http://")
        || target.starts_with("https://")
    {
        return false;
    }
    true
}

pub fn validate_overlay(overlay: &VhostOverlay) -> Result<(), OverlayPublishError> {
    if overlay.site_id.is_empty() || overlay.document_root.is_empty() {
        return Err(OverlayPublishError::InvalidOverlay);
    }
    for entry in overlay.entries.iter() {
        if !entry.directory.0.starts_with('/') {
            return Err(OverlayPublishError::InvalidOverlay);
        }
        for rule in entry.redirect_rules.iter() {
            if !rule.from_path.starts_with('/') || rule.location.is_empty() {
                return Err(OverlayPublishError::InvalidOverlay);
            }
            if rule.status != 301 && rule.status != 302 {
                return Err(OverlayPublishError::InvalidOverlay);
            }
        }
        for rule in entry.rewrite_redirects.iter() {
            if !rule.from_path.starts_with('/') || rule.location.is_empty() {
                return Err(OverlayPublishError::InvalidOverlay);
            }
            if rule.status != 301 && rule.status != 302 {
                return Err(OverlayPublishError::InvalidOverlay);
            }
        }
        if let Some(candidates) = &entry.directory_index {
            if candidates.len() > MAX_DIRECTORY_INDEX_CANDIDATES {
                return Err(OverlayPublishError::InvalidOverlay);
            }
            for name in candidates.iter() {
                if !validate_directory_index_candidate(name) {
                    return Err(OverlayPublishError::InvalidOverlay);
                }
            }
        }
        if let Some(fc) = &entry.front_controller {
            if !validate_front_controller_target(&fc.target_uri)
                || !fc.require_not_file
                || !fc.require_not_directory
            {
                return Err(OverlayPublishError::InvalidOverlay);
            }
        }
    }
    Ok(())
}

pub fn validate_publish(
    patch: &RuntimePatchVhostOverlay,
    active_plan_generation: u64,
) -> Result<(), OverlayPublishError> {
    if patch.plan_generation != active_plan_generation {
        return Err(OverlayPublishError::ObsoletePlanGeneration);
    }
    if patch.overlay.site_id != patch.site_id {
        return Err(OverlayPublishError::SiteIdMismatch);
    }
    validate_overlay(&patch.overlay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn join_and_validate_candidates() {
        assert!(validate_directory_index_candidate("index.html"));
        assert!(!validate_directory_index_candidate("../x"));
        assert_eq!(
            join_directory_index_uri("/admin/", "index.html").as_deref(),
            Some("/admin/index.html")
        );
        assert_eq!(
            join_directory_index_uri("/", "index.php").as_deref(),
            Some("/index.php")
        );
    }

    #[test]
    fn lookup_overlay_directory_index_override() {
        let overlay = VhostOverlay {
            generation: 1,
            site_id: "site".into(),
            document_root: "/var/www".into(),
            entries: Arc::from([
                OverlayEntry {
                    directory: NormalizedDirectory("/".into()),
                    directory_index: Some(Arc::from(["index.html".into()].as_slice())),
                    redirect_rules: Arc::from([]),
                    rewrite_redirects: Arc::from([]),
                    indexes_disabled: false,
                    front_controller: None,
                },
                OverlayEntry {
                    directory: NormalizedDirectory("/admin/".into()),
                    directory_index: Some(Arc::from(["admin.html".into()].as_slice())),
                    redirect_rules: Arc::from([]),
                    rewrite_redirects: Arc::from([]),
                    indexes_disabled: false,
                    front_controller: None,
                },
            ]),
        };
        if let OverlayLookupResult::Continue {
            directory_index, ..
        } = lookup_overlay("/admin/", &overlay)
        {
            assert_eq!(
                directory_index.as_ref().map(|v| v[0].as_str()),
                Some("admin.html")
            );
        } else {
            panic!("expected continue");
        }
    }

    #[test]
    fn validate_overlay_rejects_bad_directory_index() {
        let overlay = VhostOverlay {
            generation: 1,
            site_id: "site".into(),
            document_root: "/var/www".into(),
            entries: Arc::from([OverlayEntry {
                directory: NormalizedDirectory("/".into()),
                directory_index: Some(Arc::from(["../x".into()].as_slice())),
                redirect_rules: Arc::from([]),
                rewrite_redirects: Arc::from([]),
                indexes_disabled: false,
                front_controller: None,
            }]),
        };
        assert_eq!(
            validate_overlay(&overlay),
            Err(OverlayPublishError::InvalidOverlay)
        );
    }
}
