//! Size-based log rotation (IR `logging.file.rotation.max_bytes`).
//!
//! Prefer size rotation over daily for Cap061: real rollover without synthetic clocks.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub struct SizeRotatingWriter {
    dir: PathBuf,
    prefix: String,
    max_bytes: u64,
    keep: u32,
    current: File,
    path: PathBuf,
    written: u64,
    generation: u64,
}

impl SizeRotatingWriter {
    pub fn open(dir: &Path, prefix: &str, max_bytes: u64, keep: u32) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let generation = next_generation(dir, prefix);
        let path = active_path(dir, prefix);
        let current = OpenOptions::new().create(true).append(true).open(&path)?;
        let written = current.metadata()?.len();
        Ok(Self {
            dir: dir.to_path_buf(),
            prefix: prefix.to_string(),
            max_bytes: max_bytes.max(1),
            keep: keep.max(1),
            current,
            path,
            written,
            generation,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.current.flush()?;
        let archived = self
            .dir
            .join(format!("{}.{}", self.prefix, self.generation));
        // Replace active → archived; open fresh active. Rename failure must not
        // truncate the live file (LA-CAP061-008).
        if self.path.exists() {
            fs::rename(&self.path, &archived)?;
        }
        self.generation = self.generation.saturating_add(1);
        self.current = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        self.written = 0;
        prune_old(&self.dir, &self.prefix, self.keep)?;
        Ok(())
    }
}

impl Write for SizeRotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.written > 0 && self.written.saturating_add(buf.len() as u64) > self.max_bytes {
            self.rotate()?;
        }
        let n = self.current.write(buf)?;
        self.written = self.written.saturating_add(n as u64);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.current.flush()
    }
}

fn active_path(dir: &Path, prefix: &str) -> PathBuf {
    dir.join(prefix)
}

fn next_generation(dir: &Path, prefix: &str) -> u64 {
    let mut max = 0u64;
    let Ok(entries) = fs::read_dir(dir) else {
        return 1;
    };
    let marker = format!("{prefix}.");
    for ent in entries.flatten() {
        let name = ent.file_name();
        let Some(s) = name.to_str() else { continue };
        if let Some(rest) = s.strip_prefix(&marker) {
            if let Ok(n) = rest.parse::<u64>() {
                max = max.max(n);
            }
        }
    }
    max.saturating_add(1)
}

fn prune_old(dir: &Path, prefix: &str, keep: u32) -> io::Result<()> {
    let mut gens: Vec<(u64, PathBuf)> = Vec::new();
    let marker = format!("{prefix}.");
    for ent in fs::read_dir(dir)?.flatten() {
        let path = ent.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Some(rest) = name.strip_prefix(&marker) {
            if let Ok(n) = rest.parse::<u64>() {
                gens.push((n, path));
            }
        }
    }
    gens.sort_by_key(|(n, _)| *n);
    let excess = gens.len().saturating_sub(keep as usize);
    for (_, path) in gens.into_iter().take(excess) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn rotates_when_max_bytes_exceeded() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = SizeRotatingWriter::open(dir.path(), "app.log", 32, 3).unwrap();
        w.write_all(b"abcdefghijklmnopqrstuvwxyz012345").unwrap();
        w.write_all(b"MORE").unwrap();
        w.flush().unwrap();
        let active = dir.path().join("app.log");
        let active_len = std::fs::metadata(&active).unwrap().len();
        assert!(active_len >= 4, "post-rotate active holds new bytes");
        let archived: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("app.log."))
            })
            .collect();
        assert!(!archived.is_empty(), "expected archived generation file");
    }
}
