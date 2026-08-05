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
//! Core→module epoll registration hooks (KD2.3). Core installs once at pool prepare.
#![cfg_attr(target_os = "linux", allow(clippy::type_complexity, dead_code))]

#[cfg(target_os = "linux")]
use std::sync::OnceLock;
#[cfg(target_os = "linux")]
use tokio::net::TcpStream;

#[cfg(target_os = "linux")]
pub struct StaticWireEpollHooks {
    pub register_sendfile_from_tokio_first: fn(TcpStream, &[u8], &[u8]) -> Result<(), TcpStream>,
    pub register_keepalive_from_tokio_first: fn(TcpStream, &[u8], &[u8]) -> Result<(), TcpStream>,
    pub register_keepalive_from_tokio: fn(TcpStream, &[u8]) -> Result<(), TcpStream>,
    pub note_sendfile_fallback: fn(),
    pub note_sendfile_fallback_rejected: fn(),
    pub keepalive_handoff_enabled: fn() -> bool,
}

#[cfg(target_os = "linux")]
static HOOKS: OnceLock<StaticWireEpollHooks> = OnceLock::new();

#[cfg(target_os = "linux")]
pub fn install_epoll_hooks(hooks: StaticWireEpollHooks) -> Result<(), StaticWireEpollHooks> {
    HOOKS.set(hooks)
}

#[cfg(target_os = "linux")]
fn hooks() -> &'static StaticWireEpollHooks {
    HOOKS
        .get()
        .expect("StaticWireEpollHooks not installed — call install_epoll_hooks from core")
}

#[cfg(target_os = "linux")]
pub fn register_sendfile_from_tokio_first(
    stream: TcpStream,
    head: &[u8],
    carry: &[u8],
) -> Result<(), TcpStream> {
    (hooks().register_sendfile_from_tokio_first)(stream, head, carry)
}

#[cfg(target_os = "linux")]
pub fn register_keepalive_from_tokio_first(
    stream: TcpStream,
    head: &[u8],
    carry: &[u8],
) -> Result<(), TcpStream> {
    (hooks().register_keepalive_from_tokio_first)(stream, head, carry)
}

#[cfg(target_os = "linux")]
pub fn register_keepalive_from_tokio(stream: TcpStream, carry: &[u8]) -> Result<(), TcpStream> {
    (hooks().register_keepalive_from_tokio)(stream, carry)
}

#[cfg(target_os = "linux")]
pub fn note_sendfile_fallback() {
    (hooks().note_sendfile_fallback)();
}

#[cfg(target_os = "linux")]
pub fn note_sendfile_fallback_rejected() {
    (hooks().note_sendfile_fallback_rejected)();
}

#[cfg(target_os = "linux")]
pub fn keepalive_handoff_enabled() -> bool {
    (hooks().keepalive_handoff_enabled)()
}
