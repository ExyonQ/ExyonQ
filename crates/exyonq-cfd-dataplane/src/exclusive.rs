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

//! Process-exclusive lock so SO_REUSEPORT cannot admit a second dataplane owner.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

/// Held for process lifetime — drop releases flock.
pub struct DataplaneLock {
    #[cfg(unix)]
    _file: std::fs::File,
}

/// Acquire exclusive ownership of this gen-dir dataplane instance.
///
/// Prevents a second `exyonq-dataplane` from joining the same competitive
/// REUSEPORT accept group (owner freeze: dual-accept forbidden).
pub fn acquire(gen_dir: &Path) -> io::Result<DataplaneLock> {
    #[cfg(unix)]
    {
        let path = gen_dir.join("dataplane.lock");
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)?;
        if !exyonq_linux_ffi::try_flock_exclusive(file.as_raw_fd())? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "competitive H1 dataplane lock held (exclusive ownership required)",
            ));
        }
        file.set_len(0)?;
        writeln!(file, "{}", std::process::id())?;
        let _ = file.flush();
        Ok(DataplaneLock { _file: file })
    }
    #[cfg(not(unix))]
    {
        let _ = gen_dir;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "dataplane lock requires unix",
        ))
    }
}
