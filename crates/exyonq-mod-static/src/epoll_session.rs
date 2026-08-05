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
//! Module-owned epoll sendfile sessions (KD2.5 — core stores opaque session ids only).
#![cfg_attr(target_os = "linux", allow(dead_code))]

use crate::runtime::StaticRuntime;
use crate::sendfile_fsm::{epoll_drain_pump_once, PumpResult, SendingState};
use exyonq_module_api::static_dispatch::SendfileHandle;
use exyonq_module_api::static_epoll::StaticEpollPumpResult;
use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static RUNTIME: Mutex<Option<Arc<StaticRuntime>>> = Mutex::new(None);
static SESSIONS: Mutex<Option<SessionTable>> = Mutex::new(None);

struct SessionTable {
    next_id: AtomicU64,
    by_id: HashMap<u64, SendingState>,
    fd_session: HashMap<RawFd, u64>,
}

impl SessionTable {
    fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            by_id: HashMap::new(),
            fd_session: HashMap::new(),
        }
    }
}

fn sessions() -> std::sync::MutexGuard<'static, Option<SessionTable>> {
    let mut guard = SESSIONS.lock().expect("epoll sessions poisoned");
    if guard.is_none() {
        *guard = Some(SessionTable::new());
    }
    guard
}

/// Pin the registered runtime for epoll FSM hooks (composition root; tests may replace).
pub fn pin_runtime(runtime: Arc<StaticRuntime>) {
    *RUNTIME.lock().expect("epoll runtime pin poisoned") = Some(runtime);
}

fn runtime() -> Arc<StaticRuntime> {
    RUNTIME
        .lock()
        .expect("epoll runtime pin poisoned")
        .clone()
        .expect("StaticRuntime not pinned for epoll FSM")
}

/// Composition-root accessor for wire hooks.
pub(crate) fn runtime_for_hooks() -> Arc<StaticRuntime> {
    runtime()
}

pub fn match_bench_sendfile(site_slot: u32, head: &[u8]) -> Option<SendfileHandle> {
    let root = runtime().root_for_slot_public(site_slot).ok()?;
    let asset = root.match_bench_sendfile_head_arc(head)?;
    let handle = runtime().issue_sendfile_handle(asset);
    Some(handle)
}

pub fn begin_sendfile_session(
    fd: RawFd,
    head_only: bool,
    asset: SendfileHandle,
) -> Result<u64, ()> {
    let asset_arc = runtime().take_sendfile_handle(asset).ok_or(())?;
    let state = if head_only {
        SendingState::new_head_only(asset_arc).map_err(|_| ())?
    } else {
        SendingState::new_get(asset_arc).map_err(|_| ())?
    };
    let mut guard = sessions();
    let table = guard.as_mut().expect("session table");
    let id = table.next_id.fetch_add(1, Ordering::Relaxed);
    table.by_id.insert(id, state);
    table.fd_session.insert(fd, id);
    Ok(id)
}

pub fn pump_sendfile_session(fd: RawFd, session: u64) -> StaticEpollPumpResult {
    let mut guard = sessions();
    let table = guard.as_mut().expect("session table");
    let Some(state) = table.by_id.get_mut(&session) else {
        return StaticEpollPumpResult::Error;
    };
    let drain = epoll_drain_pump_once(fd, state);
    match drain.result {
        PumpResult::Progress => StaticEpollPumpResult::Progress,
        PumpResult::Parked => StaticEpollPumpResult::Parked,
        PumpResult::Complete => StaticEpollPumpResult::Complete,
        PumpResult::Error(_) => StaticEpollPumpResult::Error,
    }
}

pub fn clear_sendfile_session(fd: RawFd, session: u64) {
    let mut guard = sessions();
    let table = guard.as_mut().expect("session table");
    table.by_id.remove(&session);
    table.fd_session.remove(&fd);
}

#[cfg(any(test, feature = "test-utils"))]
pub fn clear_all_sessions_for_test() {
    let mut guard = sessions();
    if let Some(table) = guard.as_mut() {
        table.by_id.clear();
        table.fd_session.clear();
    }
}
