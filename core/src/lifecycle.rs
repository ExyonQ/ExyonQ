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
//! Generic kernel lifecycle primitives: drain, shutdown signal, connection counting.
//!
//! PS1A (L2): one accepted TCP connection → at most one successful [`LifecycleState::try_enter`]
//! → exactly one [`ConnectionLifecycleToken`] → exactly one drop decrement.
//!
//! PS1A-LINUX: admission uses increment-and-recheck (Option A) so `try_enter` is linealizable
//! with `start_drain` — no connection remains admitted after drain completes.

use std::cell::RefCell;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// Admission rejected while the server is draining (no counter increment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainRejected;

/// Per-instance test hook for the increment→recheck window (never process-global).
#[cfg(test)]
#[derive(Debug, Default)]
struct TryEnterTestHook {
    pause_after_increment: AtomicBool,
    release: AtomicBool,
}

#[derive(Debug)]
pub struct LifecycleState {
    draining: AtomicBool,
    active_connections: AtomicU64,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    /// Instance-scoped only: arming pause on one `LifecycleState` must not affect peers.
    #[cfg(test)]
    try_enter_hook: TryEnterTestHook,
}

impl LifecycleState {
    pub fn new() -> Arc<Self> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Arc::new(Self {
            draining: AtomicBool::new(false),
            active_connections: AtomicU64::new(0),
            shutdown_tx,
            shutdown_rx,
            #[cfg(test)]
            try_enter_hook: TryEnterTestHook::default(),
        })
    }

    /// Arm the post-increment pause on *this* instance only (test isolation).
    #[cfg(test)]
    fn test_arm_try_enter_pause(self: &Arc<Self>) {
        self.try_enter_hook
            .pause_after_increment
            .store(true, Ordering::Release);
        self.try_enter_hook.release.store(false, Ordering::Release);
    }

    /// Release the post-increment pause on *this* instance only.
    #[cfg(test)]
    fn test_release_try_enter_pause(self: &Arc<Self>) {
        self.try_enter_hook.release.store(true, Ordering::Release);
    }

    pub fn start_drain(self: &Arc<Self>) {
        self.draining.store(true, Ordering::Release);
    }

    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }

    pub fn request_shutdown(self: &Arc<Self>) {
        self.draining.store(true, Ordering::Release);
        let _ = self.shutdown_tx.send(true);
    }

    pub fn shutdown_requested(&self) -> bool {
        *self.shutdown_rx.borrow()
    }

    pub fn shutdown_rx(&self) -> watch::Receiver<bool> {
        self.shutdown_rx.clone()
    }

    pub async fn wait_for_shutdown(ops: &Arc<Self>) {
        if ops.shutdown_requested() {
            return;
        }
        let mut rx = ops.shutdown_rx();
        let _ = rx.changed().await;
    }

    /// Cap041 default grace: wait for admitted work to finish after shutdown is requested.
    ///
    /// Not a TOML knob — product constant. Timeout proceeds to forced exit (runtime drop).
    pub const DEFAULT_GRACEFUL_SHUTDOWN_WAIT: Duration = Duration::from_secs(30);

    /// Poll until [`Self::drain_complete`] or `timeout`.
    ///
    /// Returns `true` when `drain_complete` observed; `false` on timeout.
    pub async fn wait_for_drain_complete(ops: &Arc<Self>, timeout: Duration) -> bool {
        debug_assert!(
            ops.is_draining(),
            "wait_for_drain_complete requires drain/shutdown already requested"
        );
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if ops.drain_complete() {
                return true;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return false;
            }
            let slice = (deadline - now).min(Duration::from_millis(25));
            tokio::time::sleep(slice).await;
        }
    }

    /// Observed together with [`Self::is_draining`] for drain completion — Acquire pairs with
    /// token enter (AcqRel) and drop (Release).
    pub fn active_connections(&self) -> u64 {
        self.active_connections.load(Ordering::Acquire)
    }

    /// Admit one connection (Option A: increment and recheck).
    ///
    /// Linearization point on success: recheck observes `draining == false` after increment.
    /// Linearization point on failure: rollback decrement completes before returning `Err`.
    pub fn try_enter(self: &Arc<Self>) -> Result<ConnectionLifecycleToken, DrainRejected> {
        if self.is_draining() {
            return Err(DrainRejected);
        }
        self.active_connections.fetch_add(1, Ordering::AcqRel);

        // Instance-scoped only — process-global latches caused parallel-test flakes/hangs.
        #[cfg(test)]
        while self
            .try_enter_hook
            .pause_after_increment
            .load(Ordering::Acquire)
            && !self.try_enter_hook.release.load(Ordering::Acquire)
        {
            std::thread::yield_now();
        }

        if self.is_draining() {
            self.active_connections.fetch_sub(1, Ordering::Release);
            return Err(DrainRejected);
        }

        Ok(ConnectionLifecycleToken {
            state: Arc::clone(self),
        })
    }

    /// Returns true when graceful drain may complete (no admitted connections remain).
    pub fn drain_complete(&self) -> bool {
        self.is_draining() && self.active_connections() == 0
    }

    /// Cap031: extend admission for a Hyper WebSocket tunnel that outlives the HTTP connection future.
    ///
    /// Call only after a successful upstream 101 while the original connection is still admitted.
    /// Increments even during drain so a pre-drain tunnel remains counted until teardown
    /// (`LIVE_WEBSOCKET_TUNNEL ⇒ LIVE_LIFECYCLE_OWNERSHIP`).
    pub fn extend_for_upgraded_tunnel(self: &Arc<Self>) -> ConnectionLifecycleToken {
        self.active_connections.fetch_add(1, Ordering::AcqRel);
        ConnectionLifecycleToken {
            state: Arc::clone(self),
        }
    }
}

/// RAII connection admission: drop decrements `active_connections` exactly once.
pub struct ConnectionLifecycleToken {
    state: Arc<LifecycleState>,
}

impl ConnectionLifecycleToken {
    /// Observe drain without releasing admission (keepalive / epoll per-request gate).
    #[inline]
    pub fn is_draining(&self) -> bool {
        self.state.is_draining()
    }
}

impl Drop for ConnectionLifecycleToken {
    fn drop(&mut self) {
        self.state
            .active_connections
            .fetch_sub(1, Ordering::Release);
    }
}

tokio::task_local! {
    static TASK_CONNECTION_TOKEN: RefCell<Option<ConnectionLifecycleToken>>;
}

/// Run an async connection task with a task-local admission token (Tokio keep-alive handoff).
pub async fn run_with_connection_token<F>(token: ConnectionLifecycleToken, fut: F) -> F::Output
where
    F: Future,
{
    TASK_CONNECTION_TOKEN
        .scope(RefCell::new(Some(token)), fut)
        .await
}

/// Take the task-local token for epoll keep-alive registration (moves ownership out).
pub(crate) fn take_task_connection_token() -> Option<ConnectionLifecycleToken> {
    TASK_CONNECTION_TOKEN
        .try_with(|slot| slot.borrow_mut().take())
        .ok()
        .flatten()
}

/// Restore admission ownership to the task-local slot after a failed handoff.
///
/// Returns `Err(token)` when not inside [`run_with_connection_token`] or when the slot is
/// already occupied — caller must then end the connection with the token (Drop) rather than
/// continuing without accounting (V044-LIFECYCLE-C1).
///
/// Call site: Linux `epoll_start::register_keepalive_from_tokio_buffered` (cfg-gated).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn restore_task_connection_token(
    token: ConnectionLifecycleToken,
) -> Result<(), ConnectionLifecycleToken> {
    let mut token = Some(token);
    let placed = TASK_CONNECTION_TOKEN
        .try_with(|slot| {
            let mut guard = slot.borrow_mut();
            if guard.is_some() {
                return false;
            }
            *guard = token.take();
            true
        })
        .unwrap_or(false);
    if placed {
        Ok(())
    } else {
        Err(token.expect("admission token not placed in task-local slot"))
    }
}

/// Admit using task-local token when present, otherwise [`LifecycleState::try_enter`].
pub fn admit_connection(
    ops: &Arc<LifecycleState>,
) -> Result<ConnectionLifecycleToken, DrainRejected> {
    if let Some(token) = take_task_connection_token() {
        return Ok(token);
    }
    ops.try_enter()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::thread;

    #[test]
    fn try_enter_increments_once() {
        let ops = LifecycleState::new();
        assert_eq!(ops.active_connections(), 0);
        let _t = ops.try_enter().unwrap();
        assert_eq!(ops.active_connections(), 1);
    }

    #[test]
    fn extend_for_upgraded_tunnel_counts_during_drain() {
        let ops = LifecycleState::new();
        let _http = ops.try_enter().unwrap();
        ops.start_drain();
        let tunnel = ops.extend_for_upgraded_tunnel();
        assert_eq!(ops.active_connections(), 2);
        assert!(!ops.drain_complete());
        drop(_http);
        assert_eq!(ops.active_connections(), 1);
        assert!(!ops.drain_complete());
        drop(tunnel);
        assert_eq!(ops.active_connections(), 0);
        assert!(ops.drain_complete());
    }

    #[test]
    fn token_drop_decrements_once() {
        let ops = LifecycleState::new();
        {
            let _t = ops.try_enter().unwrap();
            assert_eq!(ops.active_connections(), 1);
        }
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn drain_rejects_new_enter() {
        let ops = LifecycleState::new();
        ops.start_drain();
        assert!(ops.try_enter().is_err());
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn failed_enter_does_not_increment() {
        let ops = LifecycleState::new();
        ops.start_drain();
        let _ = ops.try_enter();
        assert_eq!(ops.active_connections(), 0);
    }

    /// Option A rollback: one fetch_add, one rollback fetch_sub, no token Drop.
    ///
    /// Pause is instance-scoped so parallel peers cannot clear/release a process-global latch
    /// (root cause of V044_LOGIC_FLAKE_001).
    #[test]
    fn rollback_admission_decrements_exactly_once() {
        let ops = LifecycleState::new();
        ops.test_arm_try_enter_pause();
        let worker = {
            let ops = Arc::clone(&ops);
            thread::spawn(move || ops.try_enter())
        };
        while ops.active_connections() == 0 {
            thread::yield_now();
        }
        assert_eq!(ops.active_connections(), 1);
        ops.start_drain();
        assert!(!ops.drain_complete());
        ops.test_release_try_enter_pause();
        assert!(worker.join().unwrap().is_err());
        assert_eq!(ops.active_connections(), 0);
        assert!(ops.drain_complete());
    }

    /// Peer LifecycleState must not observe another instance's pause arming.
    #[test]
    fn try_enter_pause_is_instance_scoped() {
        let paused = LifecycleState::new();
        let peer = LifecycleState::new();
        paused.test_arm_try_enter_pause();
        let token = peer.try_enter().expect("peer must not share pause latch");
        assert_eq!(peer.active_connections(), 1);
        drop(token);
        assert_eq!(peer.active_connections(), 0);
        assert_eq!(paused.active_connections(), 0);
        paused.test_release_try_enter_pause();
    }

    /// Concurrent peer try_enter/reset traffic must not break instance-scoped rollback proof.
    #[test]
    fn rollback_admission_survives_parallel_peer_try_enter() {
        let ops = LifecycleState::new();
        ops.test_arm_try_enter_pause();
        let peer = LifecycleState::new();
        let peer_stop = Arc::new(AtomicBool::new(false));
        let peer_worker = {
            let peer = Arc::clone(&peer);
            let peer_stop = Arc::clone(&peer_stop);
            thread::spawn(move || {
                while !peer_stop.load(Ordering::Acquire) {
                    if let Ok(token) = peer.try_enter() {
                        drop(token);
                    }
                    thread::yield_now();
                }
            })
        };
        let worker = {
            let ops = Arc::clone(&ops);
            thread::spawn(move || ops.try_enter())
        };
        while ops.active_connections() == 0 {
            thread::yield_now();
        }
        assert_eq!(ops.active_connections(), 1);
        ops.start_drain();
        ops.test_release_try_enter_pause();
        assert!(worker.join().unwrap().is_err());
        assert_eq!(ops.active_connections(), 0);
        assert!(ops.drain_complete());
        peer_stop.store(true, Ordering::Release);
        peer_worker.join().unwrap();
        assert_eq!(peer.active_connections(), 0);
    }

    #[test]
    fn token_move_does_not_decrement_until_final_drop() {
        let ops = LifecycleState::new();
        let t1 = ops.try_enter().unwrap();
        assert_eq!(ops.active_connections(), 1);
        let t2 = t1;
        drop(t2);
        assert_eq!(ops.active_connections(), 0);
    }

    #[tokio::test]
    async fn task_local_scope_preserves_count_until_end() {
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        run_with_connection_token(token, async {
            assert_eq!(ops.active_connections(), 1);
            let taken = take_task_connection_token();
            assert!(taken.is_some());
            assert_eq!(ops.active_connections(), 1);
            drop(taken);
            assert_eq!(ops.active_connections(), 0);
        })
        .await;
        assert_eq!(ops.active_connections(), 0);
    }

    /// V044-LIFECYCLE-C1: failed handoff must restore admission ownership without undercount.
    #[tokio::test]
    async fn restore_task_local_token_preserves_active_count() {
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        run_with_connection_token(token, async {
            assert_eq!(ops.active_connections(), 1);
            let taken = take_task_connection_token().expect("task-local token");
            assert_eq!(ops.active_connections(), 1);
            assert!(
                restore_task_connection_token(taken).is_ok(),
                "restore into empty slot"
            );
            assert_eq!(ops.active_connections(), 1);
            let again = take_task_connection_token().expect("restored token");
            drop(again);
            assert_eq!(ops.active_connections(), 0);
        })
        .await;
        assert_eq!(ops.active_connections(), 0);
    }

    #[tokio::test]
    async fn admit_connection_reuses_task_local_without_second_enter() {
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        run_with_connection_token(token, async {
            let again = admit_connection(&ops).unwrap();
            drop(again);
            assert_eq!(ops.active_connections(), 0);
        })
        .await;
        assert_eq!(ops.active_connections(), 0);
    }

    #[test]
    fn drain_blocks_until_last_token_drops() {
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        ops.start_drain();
        assert_eq!(ops.active_connections(), 1);
        assert!(!ops.drain_complete());
        assert!(ops.try_enter().is_err());
        drop(token);
        assert_eq!(ops.active_connections(), 0);
        assert!(ops.drain_complete());
    }

    #[test]
    fn shutdown_signal_with_active_connection() {
        let ops = LifecycleState::new();
        let _token = ops.try_enter().unwrap();
        ops.request_shutdown();
        assert!(ops.shutdown_requested());
        assert!(ops.is_draining());
        assert_eq!(ops.active_connections(), 1);
    }

    #[tokio::test]
    async fn wait_for_drain_complete_observes_token_drop() {
        let ops = LifecycleState::new();
        let token = ops.try_enter().unwrap();
        ops.request_shutdown();
        assert!(!ops.drain_complete());
        let ops_wait = Arc::clone(&ops);
        let waiter = tokio::spawn(async move {
            LifecycleState::wait_for_drain_complete(&ops_wait, Duration::from_secs(2)).await
        });
        tokio::task::yield_now().await;
        drop(token);
        assert!(waiter.await.expect("join"));
        assert!(ops.drain_complete());
    }

    #[tokio::test]
    async fn wait_for_drain_complete_times_out_while_held() {
        let ops = LifecycleState::new();
        let _token = ops.try_enter().unwrap();
        ops.request_shutdown();
        let drained =
            LifecycleState::wait_for_drain_complete(&ops, Duration::from_millis(50)).await;
        assert!(!drained);
        assert_eq!(ops.active_connections(), 1);
    }

    #[test]
    fn concurrent_try_enter_while_drain_starts() {
        let ops = LifecycleState::new();
        let threads = 32;
        let start = Arc::new(Barrier::new(threads + 1));
        let mut handles = Vec::new();
        let successes = Arc::new(AtomicU64::new(0));
        let failures = Arc::new(AtomicU64::new(0));

        for _ in 0..threads {
            let ops = Arc::clone(&ops);
            let start = Arc::clone(&start);
            let successes = Arc::clone(&successes);
            let failures = Arc::clone(&failures);
            handles.push(thread::spawn(move || {
                start.wait();
                match ops.try_enter() {
                    Ok(token) => {
                        successes.fetch_add(1, Ordering::AcqRel);
                        drop(token);
                    }
                    Err(DrainRejected) => {
                        failures.fetch_add(1, Ordering::AcqRel);
                    }
                }
            }));
        }
        start.wait();
        ops.start_drain();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(ops.active_connections(), 0);
        assert!(ops.drain_complete());
        assert_eq!(
            successes.load(Ordering::Acquire) + failures.load(Ordering::Acquire),
            threads as u64
        );
    }

    #[test]
    fn stress_try_enter_never_underflows() {
        const ROUNDS: usize = 200;
        for _ in 0..ROUNDS {
            let ops = LifecycleState::new();
            let threads = 16;
            let barrier = Arc::new(Barrier::new(threads));
            let mut handles = Vec::new();
            for t in 0..threads {
                let ops = Arc::clone(&ops);
                let barrier = Arc::clone(&barrier);
                handles.push(thread::spawn(move || {
                    barrier.wait();
                    if t % 4 == 0 {
                        ops.start_drain();
                    }
                    let maybe = ops.try_enter();
                    std::thread::sleep(std::time::Duration::from_micros(t as u64));
                    drop(maybe);
                }));
            }
            for handle in handles {
                handle.join().unwrap();
            }
            assert_eq!(ops.active_connections(), 0);
        }
    }
}
