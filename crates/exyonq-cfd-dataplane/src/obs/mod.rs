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
//! CFD dataplane observability — Cap061-semantic local projection.
//!
//! Hot path: atomics + bounded `try_push` events (no sink I/O).
//! Export thread / scrape socket: console JSON, file(+size rotate), syslog,
//! journald, OpenMetrics. Cap061 OTLP stays on the product plane (Tokio).

mod export;
mod file_rotate;
mod hub;

pub use hub::{ObsEvent, ObsHub, ObsHubHandle, ObsRecord, StatusClass};

/// Re-exported for tests / documentation of the event ring bound.
#[allow(unused_imports)]
pub use hub::EVENT_QUEUE_BOUND;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// True when any CFD observability sink or metrics scrape is requested via env.
pub fn obs_env_configured() -> bool {
    use std::env;
    const KEYS: &[&str] = &[
        "EXYONQ_CFD_OBS_CONSOLE_JSON",
        "EXYONQ_CFD_OBS_FILE",
        "EXYONQ_CFD_OBS_SYSLOG",
        "EXYONQ_CFD_OBS_JOURNALD",
        "EXYONQ_CFD_OBS_METRICS_LISTEN",
    ];
    KEYS.iter().any(|k| match env::var(k) {
        Ok(v) => {
            let t = v.trim();
            !t.is_empty()
                && t != "0"
                && !t.eq_ignore_ascii_case("false")
                && !t.eq_ignore_ascii_case("no")
        }
        Err(_) => false,
    })
}

/// Spawn policy when observability is requested via env.
///
/// If any `EXYONQ_CFD_OBS_*` sink/scrape is configured, spawn failure is fatal.
/// Otherwise export is optional and absence is silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSpawnOutcome {
    Started,
    OptionalAbsent,
    RequiredFailed,
}

/// Pure policy seam for export-thread start (deterministic without OS spawn failure).
pub fn export_spawn_outcome(obs_configured: bool, spawn_ok: bool) -> ExportSpawnOutcome {
    match (obs_configured, spawn_ok) {
        (_, true) => ExportSpawnOutcome::Started,
        (false, false) => ExportSpawnOutcome::OptionalAbsent,
        (true, false) => ExportSpawnOutcome::RequiredFailed,
    }
}

/// Spawn the export/scrape worker. Returns `None` if the thread cannot start.
pub fn spawn_export_thread(hub: ObsHubHandle, stop: Arc<AtomicBool>) -> Option<JoinHandle<()>> {
    match thread::Builder::new()
        .name("cfd-obs-export".into())
        .spawn(move || export::run(hub, stop))
    {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("exyonq-dataplane obs export thread spawn failed: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_spawn_policy_required_fails_closed() {
        assert_eq!(
            export_spawn_outcome(true, false),
            ExportSpawnOutcome::RequiredFailed
        );
        assert_eq!(
            export_spawn_outcome(true, true),
            ExportSpawnOutcome::Started
        );
        assert_eq!(
            export_spawn_outcome(false, false),
            ExportSpawnOutcome::OptionalAbsent
        );
    }
}
