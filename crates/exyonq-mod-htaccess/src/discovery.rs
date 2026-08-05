//! Discover `.htaccess` files under an authorized document root.

use crate::limits::{
    MAX_BYTES_PER_FILE, MAX_BYTES_TOTAL_PER_VHOST, MAX_DIRECTORY_DEPTH,
    MAX_HTACCESS_FILES_PER_VHOST,
};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("document root missing: {0}")]
    MissingRoot(String),
    #[error("path escapes document root: {0}")]
    SymlinkEscape(String),
    #[error("directory depth limit ({MAX_DIRECTORY_DEPTH}) exceeded")]
    DepthLimit,
    #[error(".htaccess file count limit ({MAX_HTACCESS_FILES_PER_VHOST}) exceeded")]
    FileLimit,
    #[error("total .htaccess bytes limit ({MAX_BYTES_TOTAL_PER_VHOST}) exceeded")]
    TotalBytesLimit,
    #[error("file exceeds max bytes ({MAX_BYTES_PER_FILE}): {0}")]
    FileTooLarge(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    pub absolute: PathBuf,
    pub relative_directory: String,
    pub content: String,
}

pub fn discover_htaccess_files(
    document_root: &Path,
) -> Result<Vec<DiscoveredFile>, DiscoveryError> {
    let canonical_root = document_root
        .canonicalize()
        .map_err(|_| DiscoveryError::MissingRoot(document_root.display().to_string()))?;

    let mut out = Vec::new();
    let mut total_bytes = 0usize;
    walk(
        &canonical_root,
        &canonical_root,
        0,
        &mut out,
        &mut total_bytes,
    )?;
    out.sort_by(|a, b| a.relative_directory.cmp(&b.relative_directory));
    Ok(out)
}

fn walk(
    canonical_root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<DiscoveredFile>,
    total_bytes: &mut usize,
) -> Result<(), DiscoveryError> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Err(DiscoveryError::DepthLimit);
    }
    let htaccess = dir.join(".htaccess");
    if htaccess.is_file() {
        if out.len() >= MAX_HTACCESS_FILES_PER_VHOST {
            return Err(DiscoveryError::FileLimit);
        }
        let canonical = htaccess.canonicalize()?;
        if !canonical.starts_with(canonical_root) {
            return Err(DiscoveryError::SymlinkEscape(
                canonical.display().to_string(),
            ));
        }
        let meta = std::fs::metadata(&canonical)?;
        let len = meta.len() as usize;
        if len > MAX_BYTES_PER_FILE {
            return Err(DiscoveryError::FileTooLarge(
                canonical.display().to_string(),
            ));
        }
        *total_bytes += len;
        if *total_bytes > MAX_BYTES_TOTAL_PER_VHOST {
            return Err(DiscoveryError::TotalBytesLimit);
        }
        let content = std::fs::read_to_string(&canonical)?;
        let relative_directory = relative_dir(canonical_root, dir);
        out.push(DiscoveredFile {
            absolute: canonical,
            relative_directory,
            content,
        });
    }

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in read_dir {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let child = entry.path();
        let canonical_child = child.canonicalize()?;
        if !canonical_child.starts_with(canonical_root) {
            return Err(DiscoveryError::SymlinkEscape(
                canonical_child.display().to_string(),
            ));
        }
        walk(
            canonical_root,
            &canonical_child,
            depth + 1,
            out,
            total_bytes,
        )?;
    }
    Ok(())
}

fn relative_dir(root: &Path, dir: &Path) -> String {
    if dir == root {
        return "/".into();
    }
    let rel = dir.strip_prefix(root).unwrap_or(dir);
    let mut s = rel.to_string_lossy().replace('\\', "/");
    if !s.starts_with('/') {
        s.insert(0, '/');
    }
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}
