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
//! Process umask restore helper (Unix control-socket bind).

use std::sync::Mutex;

static UMASK_LOCK: Mutex<()> = Mutex::new(());

#[cfg(target_os = "macos")]
type ModeT = u16;
#[cfg(not(target_os = "macos"))]
type ModeT = u32;

unsafe extern "C" {
    fn umask(mask: ModeT) -> ModeT;
}

/// Run `f` with process umask temporarily set to `mask`, then restore.
///
/// Serialized so concurrent binds cannot clobber each other's umask.
pub fn with_umask<R>(mask: u32, f: impl FnOnce() -> R) -> R {
    let _guard = UMASK_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mask_t = mask as ModeT;
    let previous = unsafe {
        // SAFETY: umask is process-global; held under UMASK_LOCK for the critical section.
        umask(mask_t)
    };
    let out = f();
    unsafe {
        // SAFETY: restore previous mask under the same lock.
        umask(previous);
    }
    out
}
