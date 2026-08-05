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
//! Raw fd write helpers for static sendfile paths (no core::server dependency).

#[cfg(target_os = "linux")]
pub fn write_response_fd(fd: i32, buf: &[u8]) -> std::io::Result<()> {
    if buf.len() <= 16 * 1024 {
        write_once_fd(fd, buf)
    } else {
        write_all_fd(fd, buf)
    }
}

#[cfg(target_os = "linux")]
fn write_once_fd(fd: i32, buf: &[u8]) -> std::io::Result<()> {
    let written = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };
    if written < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if written as usize != buf.len() {
        write_all_fd(fd, &buf[written as usize..])
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn write_all_fd(fd: i32, mut buf: &[u8]) -> std::io::Result<()> {
    while !buf.is_empty() {
        let written = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };
        if written < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if written == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "write returned 0",
            ));
        }
        buf = &buf[written as usize..];
    }
    Ok(())
}
