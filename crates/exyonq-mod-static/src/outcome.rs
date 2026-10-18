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
//! Convert module responses ↔ contract outcomes (transport-agnostic).

use crate::sendfile_handle::SendfileHandleRegistry;
use crate::{StaticError, StaticRoot};
use exyonq_module_api::static_dispatch::{
    SendfileHandle, StaticDispatchBody, StaticDispatchOutcome, StaticMethod,
    StaticResourceIdentitySnapshot,
};
use std::io::{Read, Seek, SeekFrom};

pub fn static_error_outcome(err: StaticError) -> StaticDispatchOutcome {
    match err {
        StaticError::NotFound => StaticDispatchOutcome {
            status: 404,
            headers: Vec::new(),
            body: StaticDispatchBody::Inline(b"not found".to_vec()),
        },
        StaticError::PathTraversal => StaticDispatchOutcome {
            status: 403,
            headers: Vec::new(),
            body: StaticDispatchBody::Inline(b"forbidden".to_vec()),
        },
        StaticError::BudgetExceeded => StaticDispatchOutcome {
            // Transport-agnostic refuse-to-materialize signal. Protocol adapters
            // (H3) remap via MATERIALIZATION_BUDGET_EXCEEDED_HEADER to their
            // oversized policy (HTTP 500 / stream-local / KEEP). Status here is
            // a shell default for Hyper when no adapter remaps — not H3 policy.
            status: 500,
            headers: vec![(
                exyonq_module_api::static_dispatch::MATERIALIZATION_BUDGET_EXCEEDED_HEADER
                    .to_string(),
                "1".to_string(),
            )],
            body: StaticDispatchBody::Empty,
        },
        StaticError::Io(_) => StaticDispatchOutcome {
            status: 500,
            headers: Vec::new(),
            body: StaticDispatchBody::Inline(b"read error".to_vec()),
        },
    }
}

/// Hyper/Tokio adapter: materialize contract outcome (including sendfile handle take).
pub fn materialize_outcome(
    outcome: StaticDispatchOutcome,
    handles: &SendfileHandleRegistry,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    match outcome.body {
        StaticDispatchBody::Empty => (outcome.status, outcome.headers, Vec::new()),
        StaticDispatchBody::Inline(body) => (outcome.status, outcome.headers, body),
        StaticDispatchBody::SendfileHandle(handle) => {
            materialize_sendfile(outcome.status, outcome.headers, handle, handles)
        }
    }
}

fn materialize_sendfile(
    status: u16,
    headers: Vec<(String, String)>,
    handle: SendfileHandle,
    handles: &SendfileHandleRegistry,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let Some(asset) = handles.take(handle) else {
        return (500, Vec::new(), b"stale sendfile handle".to_vec());
    };
    let mut file = asset.file.as_ref().try_clone().expect("file clone");
    let mut body = vec![0u8; asset.body_len];
    if file.seek(SeekFrom::Start(0)).is_err() || file.read_exact(&mut body).is_err() {
        return (500, Vec::new(), b"read error".to_vec());
    }
    (status, headers, body)
}

/// Cap061 LA-CAP061-008 removed filename-keyed Hyper sendfile divert.
/// Cap067 restores zero-copy via the epoll FSM (not this Hyper materialize path).
/// Hyper continues ordinary read I/O; epoll engagement is WAF → register_sendfile.
#[cfg(target_os = "linux")]
pub fn try_sendfile_outcome(
    _root: &StaticRoot,
    _method: StaticMethod,
    _request_path: &str,
    _generation: u64,
    _handles: &SendfileHandleRegistry,
) -> Option<StaticDispatchOutcome> {
    None
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
pub fn try_sendfile_outcome(
    _root: &StaticRoot,
    _method: StaticMethod,
    _request_path: &str,
    _generation: u64,
    _handles: &SendfileHandleRegistry,
) -> Option<StaticDispatchOutcome> {
    None
}

pub fn identity_to_snapshot(
    identity: crate::StaticResourceIdentity,
) -> StaticResourceIdentitySnapshot {
    StaticResourceIdentitySnapshot {
        canonical_path: identity.canonical_path.to_string_lossy().into_owned(),
        file_len: identity.file_len,
        modified_unix_secs: identity.modified.and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs())
        }),
        modified_subsec_nanos: identity.modified.and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.subsec_nanos())
        }),
        #[cfg(unix)]
        dev: identity.platform_file_id.map(|id| id.dev),
        #[cfg(unix)]
        ino: identity.platform_file_id.map(|id| id.ino),
    }
}
