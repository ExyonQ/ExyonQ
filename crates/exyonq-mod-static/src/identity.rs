//! Plan 12 tranche 3 — static file identity for response cache revalidation.

use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

/// Platform-specific file identity (inode/device on Unix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    #[cfg(unix)]
    pub dev: u64,
    #[cfg(unix)]
    pub ino: u64,
}

/// Snapshot of a regular file used to detect modify/rename/delete without re-reading body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticResourceIdentity {
    pub canonical_path: Arc<Path>,
    pub file_len: u64,
    pub modified: Option<SystemTime>,
    pub platform_file_id: Option<FileIdentity>,
}

impl StaticResourceIdentity {
    pub fn capture(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        if !meta.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a regular file",
            ));
        }
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        Ok(Self::from_metadata(&canonical, &meta))
    }

    pub fn from_metadata(path: &Path, meta: &std::fs::Metadata) -> Self {
        Self {
            canonical_path: Arc::from(path),
            file_len: meta.len(),
            modified: meta.modified().ok(),
            platform_file_id: platform_file_id(meta),
        }
    }

    /// Fail closed: metadata errors or identity drift → false.
    pub fn matches_current(&self) -> bool {
        match std::fs::metadata(self.canonical_path.as_ref()) {
            Ok(meta) if meta.is_file() => Self::from_metadata(&self.canonical_path, &meta) == *self,
            _ => false,
        }
    }
}

#[cfg(unix)]
fn platform_file_id(meta: &std::fs::Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    Some(FileIdentity {
        dev: meta.dev(),
        ino: meta.ino(),
    })
}

#[cfg(not(unix))]
fn platform_file_id(_meta: &std::fs::Metadata) -> Option<FileIdentity> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    #[test]
    fn capture_detects_modification() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("f.txt");
        fs::write(&path, b"v1").expect("write");
        let id = StaticResourceIdentity::capture(&path).expect("capture");
        assert!(id.matches_current());
        fs::write(&path, b"v2-longer").expect("rewrite");
        assert!(!id.matches_current());
    }

    #[test]
    fn capture_detects_delete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gone.txt");
        fs::write(&path, b"x").expect("write");
        let id = StaticResourceIdentity::capture(&path).expect("capture");
        fs::remove_file(&path).expect("remove");
        assert!(!id.matches_current());
    }

    #[test]
    fn atomic_rename_changes_identity_on_unix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("index.html");
        let new_file = dir.path().join("index.new");
        fs::write(&target, b"old").expect("write old");
        let id = StaticResourceIdentity::capture(&target).expect("capture");
        fs::write(&new_file, b"new").expect("write new");
        fs::rename(&new_file, &target).expect("rename");
        std::thread::sleep(Duration::from_millis(10));
        let after = StaticResourceIdentity::capture(&target).expect("recapture");
        if cfg!(unix) {
            assert_ne!(id.platform_file_id, after.platform_file_id);
        }
        assert!(!id.matches_current());
    }
}
