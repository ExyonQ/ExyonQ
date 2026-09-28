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
//! Cap067 / proxy-wire cheap module hooks (ratelimit admit + metrics record).
//!
//! Wire-cheap modules must not force the full Hyper CrossCuttingPipeline.
//! OpenMetrics scrape stays Hyper (LA-CAP054-008). Compression stays Hyper.

use std::sync::OnceLock;

/// Result of a wire-path rate-limit admit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireAdmit {
    Allow,
    /// Caller must write HTTP 429 with this Retry-After (seconds, ≥1).
    Reject429 {
        retry_after_secs: u64,
    },
}

/// Optional sync hooks installed by ratelimit / metrics at pipeline build.
pub struct WireModuleHooks {
    /// Pre-request admit keyed by peer / client IP (never request XFF).
    pub admit: fn(client_ip: &str) -> WireAdmit,
    /// False when admit cannot reject, so callers skip formatting the peer IP.
    pub admit_active: fn() -> bool,
    /// Post-response counter bump (status class only — no scrape).
    pub record_response: fn(status: u16),
    /// Counter, route label, and latency for a wire/Cap067 response that never
    /// enters the module `on_response` hook.
    pub record_exchange: fn(method: &str, path: &str, status: u16, elapsed_ms: f64),
}

static HOOKS: OnceLock<WireModuleHooks> = OnceLock::new();

pub fn install_wire_module_hooks(hooks: WireModuleHooks) -> Result<(), WireModuleHooks> {
    HOOKS.set(hooks)
}

fn hooks() -> Option<&'static WireModuleHooks> {
    HOOKS.get()
}

/// Admit a wire/Cap067 request. No-op Allow when hooks are not installed.
#[inline]
pub fn wire_admit(client_ip: &str) -> WireAdmit {
    match hooks() {
        Some(h) => (h.admit)(client_ip),
        None => WireAdmit::Allow,
    }
}

/// True only when the installed admit hook can return [`WireAdmit::Reject429`].
///
/// Callers use this to skip peer-IP formatting on the static hot path.
#[inline]
pub fn wire_admit_active() -> bool {
    match hooks() {
        Some(h) => (h.admit_active)(),
        None => false,
    }
}

/// Record a completed wire/Cap067 response. No-op when hooks are not installed.
#[inline]
pub fn wire_record_response(status: u16) {
    if let Some(h) = hooks() {
        (h.record_response)(status);
    }
}

/// Record method, path, status, and latency for a completed wire response.
pub fn wire_record_exchange(method: &str, path: &str, status: u16, elapsed_ms: f64) {
    if let Some(h) = hooks() {
        (h.record_exchange)(method, path, status, elapsed_ms);
    }
}

/// HTTP/1.1 429 wire for Cap055 deny (Connection: close — fail closed).
pub fn rate_limit_reject_wire(retry_after_secs: u64) -> Vec<u8> {
    let retry = retry_after_secs.max(1);
    let body = b"rate limit exceeded";
    format!(
        "HTTP/1.1 429 Too Many Requests\r\n\
         Retry-After: {retry}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body.iter().copied())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_wire_contains_429_and_retry() {
        let w = rate_limit_reject_wire(3);
        let s = String::from_utf8_lossy(&w);
        assert!(s.starts_with("HTTP/1.1 429"));
        assert!(s.contains("Retry-After: 3"));
        assert!(s.contains("rate limit exceeded"));
    }

    #[test]
    fn wire_admit_inactive_without_hooks() {
        assert!(!wire_admit_active());
    }
}
