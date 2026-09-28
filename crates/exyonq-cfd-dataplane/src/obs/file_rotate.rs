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
//! Size-based rotating file writer (Cap061 rotation semantics, tokio-free).

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
        let path = dir.join(prefix);
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
        prune_old(&self.dir, &self.prefix, self.keep)
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
    if let Ok(entries) = fs::read_dir(dir) {
        for ent in entries.flatten() {
            let path = ent.path();
            let name = ent.file_name();
            let Some(s) = name.to_str() else { continue };
            if let Some(rest) = s.strip_prefix(&marker) {
                if let Ok(n) = rest.parse::<u64>() {
                    gens.push((n, path));
                }
            }
        }
    }
    gens.sort_by_key(|(n, _)| *n);
    while gens.len() > keep as usize {
        if let Some((_, path)) = gens.first() {
            let _ = fs::remove_file(path);
            gens.remove(0);
        } else {
            break;
        }
    }
    Ok(())
}
