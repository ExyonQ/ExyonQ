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
//! Config filesystem watcher — debounced reload requests via `KernelControlPort`.

use exyonq_module_api::kernel_control::KernelControlPort;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Debounce window preserved from core pre-KD4.14 watcher.
pub const CONFIG_RELOAD_DEBOUNCE_MS: u64 = 300;

pub fn is_relevant_event(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) | EventKind::Any
    )
}

/// Only act on events that name the watched config path (or a same-dir rename peer).
///
/// The watcher observes the parent directory (needed for atomic replace via rename).
/// Without this filter, unrelated `/tmp` traffic would request continuous reloads.
pub fn event_targets_config(event: &notify::Event, config_path: &Path) -> bool {
    let Some(cfg_name) = config_path.file_name() else {
        return event.paths.iter().any(|p| p == config_path);
    };
    let cfg_parent = config_path.parent();
    event.paths.iter().any(|p| {
        if p == config_path {
            return true;
        }
        p.file_name() == Some(cfg_name) && p.parent() == cfg_parent
    })
}

pub(crate) async fn run_debounced_reload_loop(
    config_path: PathBuf,
    port: Arc<dyn KernelControlPort>,
    cancel: CancellationToken,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<()>,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            msg = rx.recv() => {
                if msg.is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(CONFIG_RELOAD_DEBOUNCE_MS)).await;
                while rx.try_recv().is_ok() {}
                if cancel.is_cancelled() {
                    break;
                }
                let outcome = port.request_reload(&config_path).await;
                if outcome.ok {
                    info!(
                        path = %config_path.display(),
                        generation = outcome.snapshot.generation,
                        fingerprint = %outcome.snapshot.fingerprint,
                        "config reload applied"
                    );
                } else {
                    warn!(
                        path = %config_path.display(),
                        error = outcome.error.as_deref().unwrap_or("unknown"),
                        "config reload rejected"
                    );
                }
            }
        }
    }
}

pub fn spawn_config_watcher(
    config_path: PathBuf,
    port: Arc<dyn KernelControlPort>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let watch_dir = config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| config_path.clone());
        let watched = config_path.clone();
        let mut watcher = match RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    if is_relevant_event(event.kind) && event_targets_config(&event, &watched) {
                        let _ = tx.send(());
                    }
                }
            },
            notify::Config::default(),
        ) {
            Ok(w) => w,
            Err(err) => {
                warn!(%err, "config watcher unavailable");
                return;
            }
        };
        if watcher
            .watch(&watch_dir, RecursiveMode::NonRecursive)
            .is_err()
        {
            warn!(dir = %watch_dir.display(), "config watcher could not watch directory");
            return;
        }

        run_debounced_reload_loop(config_path, port, cancel, rx).await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use exyonq_module_api::kernel_control::{
        KernelControlPort, KernelStatusSnapshot, OpsCommand, OpsCommandOutcome,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct StubPort {
        reloads: Mutex<u32>,
        reject: Mutex<bool>,
    }

    #[async_trait]
    impl KernelControlPort for StubPort {
        async fn request_reload(&self, _config_path: &Path) -> OpsCommandOutcome {
            if *self.reject.lock().unwrap() {
                return OpsCommandOutcome::from_parts(
                    false,
                    OpsCommand::Reload,
                    snap(1),
                    // No stable EXY-RELOAD-* id for shutdown-gate rejects; do not invent 0010.
                    Some("reload rejected: shutdown in progress".into()),
                    None,
                );
            }
            *self.reloads.lock().unwrap() += 1;
            OpsCommandOutcome::from_parts(true, OpsCommand::Reload, snap(2), None, None)
        }

        fn request_drain(&self) -> OpsCommandOutcome {
            OpsCommandOutcome::from_parts(true, OpsCommand::Drain, snap(1), None, None)
        }

        fn request_shutdown(&self) -> OpsCommandOutcome {
            OpsCommandOutcome::from_parts(true, OpsCommand::Shutdown, snap(1), None, None)
        }

        fn read_status(&self, _include_version: bool) -> OpsCommandOutcome {
            OpsCommandOutcome::from_parts(true, OpsCommand::Status, snap(1), None, None)
        }
    }

    fn snap(generation: u64) -> KernelStatusSnapshot {
        KernelStatusSnapshot {
            generation,
            fingerprint: "test".into(),
            uptime_s: 0,
            active_connections: 0,
            draining: false,
            version: None,
            reload_in_progress: false,
        }
    }

    #[tokio::test]
    async fn debounce_coalesces_burst() {
        let port = Arc::new(StubPort::default());
        let cancel = CancellationToken::new();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let path = PathBuf::from("/tmp/exyonq.toml");
        let handle = tokio::spawn(run_debounced_reload_loop(
            path,
            port.clone(),
            cancel.clone(),
            rx,
        ));
        for _ in 0..5 {
            tx.send(()).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(CONFIG_RELOAD_DEBOUNCE_MS + 100)).await;
        assert_eq!(*port.reloads.lock().unwrap(), 1);
        cancel.cancel();
        handle.abort();
    }

    #[tokio::test]
    async fn debounce_honors_shutdown_cancel() {
        let port = Arc::new(StubPort::default());
        let cancel = CancellationToken::new();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = tokio::spawn(run_debounced_reload_loop(
            PathBuf::from("/tmp/exyonq.toml"),
            port.clone(),
            cancel.clone(),
            rx,
        ));
        cancel.cancel();
        tx.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(CONFIG_RELOAD_DEBOUNCE_MS + 100)).await;
        assert_eq!(*port.reloads.lock().unwrap(), 0);
        handle.abort();
    }

    #[tokio::test]
    async fn debounce_still_calls_port_when_reload_rejected() {
        let port = Arc::new(StubPort {
            reject: Mutex::new(true),
            ..Default::default()
        });
        let cancel = CancellationToken::new();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = tokio::spawn(run_debounced_reload_loop(
            PathBuf::from("/tmp/exyonq.toml"),
            port.clone(),
            cancel.clone(),
            rx,
        ));
        tx.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(CONFIG_RELOAD_DEBOUNCE_MS + 100)).await;
        assert_eq!(*port.reloads.lock().unwrap(), 0);
        cancel.cancel();
        handle.abort();
    }

    #[tokio::test]
    async fn stub_reload_outcomes_expose_consistent_codes() {
        let ok_port = StubPort::default();
        let success = ok_port.request_reload(Path::new("/tmp/ok.toml")).await;
        assert!(success.ok);
        assert_eq!(success.code, None);
        assert!(success.code_result_consistent());

        let reject_port = StubPort {
            reject: Mutex::new(true),
            ..Default::default()
        };
        let rejected = reject_port.request_reload(Path::new("/tmp/rej.toml")).await;
        assert!(!rejected.ok);
        assert_eq!(rejected.code, None);
        assert!(rejected.code_result_consistent());

        let no_op = OpsCommandOutcome::from_parts(
            true,
            OpsCommand::Reload,
            snap(1),
            Some("EXY-RELOAD-0008: identical config — NO_OP".into()),
            None,
        );
        assert_eq!(no_op.code.as_deref(), Some("EXY-RELOAD-0008"));
        assert!(no_op.code_result_consistent());

        let failed = OpsCommandOutcome::from_parts(
            false,
            OpsCommand::Reload,
            snap(1),
            Some("EXY-RELOAD-0003: compile rejected".into()),
            None,
        );
        assert_eq!(failed.code.as_deref(), Some("EXY-RELOAD-0003"));
        assert!(failed.code_result_consistent());

        let restart = OpsCommandOutcome::from_parts(
            false,
            OpsCommand::Reload,
            snap(1),
            Some("EXY-RELOAD-0005: restart required".into()),
            None,
        );
        assert_eq!(restart.code.as_deref(), Some("EXY-RELOAD-0005"));
        assert!(restart.code_result_consistent());
    }

    /// DB5 Part E: dropping the receiver stops further queue growth from mattering.
    #[tokio::test]
    async fn db5_unbounded_sender_errors_after_receiver_drop() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        drop(rx);
        assert!(
            tx.send(()).is_err(),
            "unbounded send must fail when receiver is closed"
        );
    }

    /// DB5 Part E: large burst remains coalesced to one reload by debounce loop.
    #[tokio::test]
    async fn db5_unbounded_burst_coalesces_under_debounce() {
        let port = Arc::new(StubPort::default());
        let cancel = CancellationToken::new();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = tokio::spawn(run_debounced_reload_loop(
            PathBuf::from("/tmp/exyonq.toml"),
            port.clone(),
            cancel.clone(),
            rx,
        ));
        for _ in 0..10_000 {
            tx.send(()).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(CONFIG_RELOAD_DEBOUNCE_MS + 150)).await;
        assert_eq!(
            *port.reloads.lock().unwrap(),
            1,
            "debounce must coalesce large control bursts"
        );
        cancel.cancel();
        handle.abort();
    }
}
