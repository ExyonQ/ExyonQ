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
//! Q-08 / ADR-060 — actionable FastCGI pool saturation diagnostics.

use crate::fcgi_stream::PoolEndpoint;

/// Timeout / busy class reported in the saturation warn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaturationKind {
    Checkout,
    Drain,
    Capacity,
    Concurrency,
    Closed,
}

impl SaturationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SaturationKind::Checkout => "checkout",
            SaturationKind::Drain => "drain",
            SaturationKind::Capacity => "capacity",
            SaturationKind::Concurrency => "concurrency",
            SaturationKind::Closed => "closed",
        }
    }

    pub fn recommendation(self) -> &'static str {
        match self {
            SaturationKind::Checkout => {
                "ExyonQ fail-fast (no unbounded queue): set max_concurrency >= php-fpm pm.max_children; raise checkout_timeout_ms only for short bursts, or speed up PHP-FPM"
            }
            SaturationKind::Drain => "wait for reload drain to finish; do not send new FastCGI work",
            SaturationKind::Capacity => {
                "raise max_connections (≤ max_concurrency) or reduce concurrent FastCGI load"
            }
            SaturationKind::Concurrency => {
                "raise [[fcgi_pool]].max_concurrency so it is >= php-fpm pm.max_children; ExyonQ fail-fast has no unbounded queue"
            }
            SaturationKind::Closed => "pool is closed/draining; retry after reload completes",
        }
    }
}

/// Format endpoint for logs (no secrets).
pub fn endpoint_display(endpoint: &PoolEndpoint) -> String {
    match endpoint {
        PoolEndpoint::Unix(p) => format!("unix:{}", p.display()),
        PoolEndpoint::Tcp(addr) => format!("tcp:{addr}"),
    }
}

/// Human one-liner used by ConnPool busy paths (also unit-tested).
pub fn format_saturation_message(
    pool: &str,
    kind: SaturationKind,
    open: usize,
    idle: usize,
    waiters: usize,
    max_connections: usize,
    endpoint: &str,
) -> String {
    format!(
        "fcgi pool saturated: pool={pool} timeout_kind={} open_sockets={open} idle_sockets={idle} waiters={waiters} max_connections={max_connections} endpoint={endpoint} queue=disabled recommendation={}",
        kind.as_str(),
        kind.recommendation()
    )
}

/// Message for the logical-concurrency semaphore, which rejects before the
/// connection pool can emit its richer waiters/sockets diagnostic.
pub fn format_concurrency_message(pool: &str, max_concurrency: u32) -> String {
    format!(
        "fcgi pool saturated: pool={pool} timeout_kind=concurrency max_concurrency={max_concurrency} queue=disabled recommendation={}",
        SaturationKind::Concurrency.recommendation()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_names_timeout_and_recommendation() {
        let msg = format_saturation_message(
            "php",
            SaturationKind::Checkout,
            4,
            0,
            2,
            4,
            "unix:/run/php.sock",
        );
        assert!(msg.contains("timeout_kind=checkout"));
        assert!(msg.contains("pool=php"));
        assert!(msg.contains("open_sockets=4"));
        assert!(msg.contains("recommendation="));
        assert!(msg.contains("max_concurrency"));
        assert!(msg.contains("queue=disabled"));
    }

    #[test]
    fn concurrency_message_names_limit_and_policy() {
        let msg = format_concurrency_message("php", 16);
        assert!(msg.contains("pool=php"));
        assert!(msg.contains("timeout_kind=concurrency"));
        assert!(msg.contains("max_concurrency=16"));
        assert!(msg.contains("pm.max_children"));
        assert!(msg.contains("queue=disabled"));
    }
}
