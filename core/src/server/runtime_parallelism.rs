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
//! Coordinated default worker geometry (V044_P1_DEFAULT_WORKER_GEOMETRY_8).
//!
//! Single source of truth for unresolved worker counts:
//!
//! ```text
//! DEFAULT_RUNTIME_PARALLELISM = available_parallelism().clamp(MIN, MAX)
//!   → Tokio workers          (EXYONQ_WORKER_THREADS unset)
//!   → accept workers         (EXYONQ_ACCEPT_WORKERS unset / auto)
//!   → Cap067 epoll pool      (EXYONQ_EPOLL_POOL_THREADS unset → follow accept)
//! ```
//!
//! Explicit env overrides remain honored. Cap067 pool is still capped by the
//! accept-worker request passed into pool prepare (`min(pool, accept)`).
//!
//! Linux `available_parallelism` respects CPU affinity / cpuset for the process;
//! it is not a blind host-socket count.

/// Inclusive minimum for any worker geometry knob.
pub const MIN_WORKERS: usize = 1;
/// Inclusive maximum for Tokio / accept / Cap067 pool defaults and clamps.
pub const MAX_WORKERS: usize = 16;

/// Detected process CPU parallelism (affinity / cpuset aware on Linux).
pub fn detected_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(MIN_WORKERS)
}

/// Canonical default when an env override is absent.
pub fn default_runtime_parallelism() -> usize {
    detected_parallelism().clamp(MIN_WORKERS, MAX_WORKERS)
}

fn parse_positive_usize(raw: &str) -> Option<usize> {
    let n: usize = raw.trim().parse().ok()?;
    if n == 0 {
        return None;
    }
    Some(n)
}

/// `EXYONQ_WORKER_THREADS` — Tokio multi-thread worker count.
pub fn resolve_tokio_worker_threads() -> usize {
    if let Ok(raw) = std::env::var("EXYONQ_WORKER_THREADS") {
        if let Some(n) = parse_positive_usize(&raw) {
            return n.clamp(MIN_WORKERS, MAX_WORKERS);
        }
    }
    default_runtime_parallelism()
}

/// `EXYONQ_ACCEPT_WORKERS` — accept / Cap067 prepare request count.
///
/// - unset → [`default_runtime_parallelism`]
/// - `auto` → same as default
/// - positive integer → clamped
pub fn resolve_accept_workers() -> usize {
    if let Ok(raw) = std::env::var("EXYONQ_ACCEPT_WORKERS") {
        if raw.eq_ignore_ascii_case("auto") {
            return default_runtime_parallelism();
        }
        if let Some(n) = parse_positive_usize(&raw) {
            return n.clamp(MIN_WORKERS, MAX_WORKERS);
        }
    }
    default_runtime_parallelism()
}

/// `EXYONQ_EPOLL_POOL_THREADS` — Cap067 keepalive OS-thread pool.
///
/// When unset, follows `accept_workers` so Cap067 cannot silently remain at a
/// historical hardcoded 4 while accept/Tokio scale.
pub fn resolve_epoll_pool_threads(accept_workers: usize) -> usize {
    let accept = accept_workers.max(MIN_WORKERS);
    let desired = if let Ok(raw) = std::env::var("EXYONQ_EPOLL_POOL_THREADS") {
        if raw.trim().is_empty() {
            accept
        } else if let Some(n) = parse_positive_usize(&raw) {
            n.clamp(MIN_WORKERS, MAX_WORKERS)
        } else {
            accept
        }
    } else {
        accept
    };
    desired.min(accept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env(pairs: &[(&str, Option<&str>)], f: impl FnOnce()) {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let keys = [
            "EXYONQ_WORKER_THREADS",
            "EXYONQ_ACCEPT_WORKERS",
            "EXYONQ_EPOLL_POOL_THREADS",
        ];
        let saved: Vec<(String, Option<String>)> = keys
            .iter()
            .map(|k| ((*k).to_string(), std::env::var(k).ok()))
            .collect();
        for k in keys {
            std::env::remove_var(k);
        }
        for (k, v) in pairs {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        f();
        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(&k, val),
                None => std::env::remove_var(&k),
            }
        }
    }

    #[test]
    fn d1_no_overrides_accept_follows_detected() {
        with_env(
            &[
                ("EXYONQ_WORKER_THREADS", None),
                ("EXYONQ_ACCEPT_WORKERS", None),
                ("EXYONQ_EPOLL_POOL_THREADS", None),
            ],
            || {
                let d = default_runtime_parallelism();
                assert_eq!(resolve_tokio_worker_threads(), d);
                assert_eq!(resolve_accept_workers(), d);
                assert_eq!(resolve_epoll_pool_threads(d), d);
            },
        );
    }

    #[test]
    fn d2_explicit_444_honored() {
        with_env(
            &[
                ("EXYONQ_WORKER_THREADS", Some("4")),
                ("EXYONQ_ACCEPT_WORKERS", Some("4")),
                ("EXYONQ_EPOLL_POOL_THREADS", Some("4")),
            ],
            || {
                assert_eq!(resolve_tokio_worker_threads(), 4);
                assert_eq!(resolve_accept_workers(), 4);
                assert_eq!(resolve_epoll_pool_threads(4), 4);
            },
        );
    }

    #[test]
    fn d3_explicit_888_honored() {
        with_env(
            &[
                ("EXYONQ_WORKER_THREADS", Some("8")),
                ("EXYONQ_ACCEPT_WORKERS", Some("8")),
                ("EXYONQ_EPOLL_POOL_THREADS", Some("8")),
            ],
            || {
                assert_eq!(resolve_tokio_worker_threads(), 8);
                assert_eq!(resolve_accept_workers(), 8);
                assert_eq!(resolve_epoll_pool_threads(8), 8);
            },
        );
    }

    #[test]
    fn d4_partial_worker_threads_only_accept_and_pool_default() {
        with_env(
            &[
                ("EXYONQ_WORKER_THREADS", Some("8")),
                ("EXYONQ_ACCEPT_WORKERS", None),
                ("EXYONQ_EPOLL_POOL_THREADS", None),
            ],
            || {
                assert_eq!(resolve_tokio_worker_threads(), 8);
                let aw = resolve_accept_workers();
                assert_eq!(aw, default_runtime_parallelism());
                assert_eq!(resolve_epoll_pool_threads(aw), aw);
            },
        );
    }

    #[test]
    fn d4_partial_accept_without_pool_pool_follows_accept() {
        with_env(
            &[
                ("EXYONQ_WORKER_THREADS", None),
                ("EXYONQ_ACCEPT_WORKERS", Some("8")),
                ("EXYONQ_EPOLL_POOL_THREADS", None),
            ],
            || {
                assert_eq!(resolve_accept_workers(), 8);
                // Historical bug: pool defaulted to 4 → Cap067 stayed at 4.
                assert_eq!(resolve_epoll_pool_threads(8), 8);
            },
        );
    }

    #[test]
    fn d5_epoll_cannot_exceed_accept() {
        with_env(
            &[
                ("EXYONQ_ACCEPT_WORKERS", Some("2")),
                ("EXYONQ_EPOLL_POOL_THREADS", Some("8")),
            ],
            || {
                assert_eq!(resolve_accept_workers(), 2);
                assert_eq!(resolve_epoll_pool_threads(2), 2);
            },
        );
    }

    #[test]
    fn auto_accept_equals_default() {
        with_env(&[("EXYONQ_ACCEPT_WORKERS", Some("auto"))], || {
            assert_eq!(resolve_accept_workers(), default_runtime_parallelism());
        });
    }

    #[test]
    fn invalid_zero_falls_back() {
        with_env(
            &[
                ("EXYONQ_WORKER_THREADS", Some("0")),
                ("EXYONQ_ACCEPT_WORKERS", Some("0")),
                ("EXYONQ_EPOLL_POOL_THREADS", Some("0")),
            ],
            || {
                let d = default_runtime_parallelism();
                assert_eq!(resolve_tokio_worker_threads(), d);
                assert_eq!(resolve_accept_workers(), d);
                assert_eq!(resolve_epoll_pool_threads(d), d);
            },
        );
    }

    #[test]
    fn clamp_above_max() {
        with_env(
            &[
                ("EXYONQ_WORKER_THREADS", Some("64")),
                ("EXYONQ_ACCEPT_WORKERS", Some("99")),
                ("EXYONQ_EPOLL_POOL_THREADS", Some("99")),
            ],
            || {
                assert_eq!(resolve_tokio_worker_threads(), MAX_WORKERS);
                assert_eq!(resolve_accept_workers(), MAX_WORKERS);
                assert_eq!(resolve_epoll_pool_threads(MAX_WORKERS), MAX_WORKERS);
            },
        );
    }

    #[test]
    fn empty_epoll_env_follows_accept() {
        with_env(&[("EXYONQ_EPOLL_POOL_THREADS", Some(""))], || {
            assert_eq!(resolve_epoll_pool_threads(8), 8);
        });
    }
}
